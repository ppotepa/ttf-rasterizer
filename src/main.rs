use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use clap::{ArgGroup, Args, Parser, Subcommand};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Print, Stylize},
    terminal::{
        self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use fontdue::{Font, FontSettings};
use reqwest::blocking::Client;
use reqwest::header::USER_AGENT;
use serde::Deserialize;
use std::{
    cmp,
    collections::{HashMap, HashSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const PREVIEW_SAMPLE_TEXT: &str = "SHELL QUEST\nmade by pawel potepa";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RasterProfile {
    Classic,
    Dense,
    Binary,
    Inverted,
}

impl RasterProfile {
    fn all() -> &'static [RasterProfile] {
        &[
            RasterProfile::Classic,
            RasterProfile::Dense,
            RasterProfile::Binary,
            RasterProfile::Inverted,
        ]
    }

    fn id(self) -> &'static str {
        match self {
            RasterProfile::Classic => "classic",
            RasterProfile::Dense => "dense",
            RasterProfile::Binary => "binary",
            RasterProfile::Inverted => "inverted",
        }
    }

    fn ramp(self) -> &'static str {
        match self {
            RasterProfile::Classic => " .:-=+*#%@",
            RasterProfile::Dense => {
                " .'`^\",:;Il!i~+_-?][}{1)(|\\/*tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$"
            }
            RasterProfile::Binary => " #",
            RasterProfile::Inverted => "@%#*+=-:. ",
        }
    }

    fn from_id(value: &str) -> Option<Self> {
        Self::all().iter().copied().find(|p| p.id() == value)
    }

    fn next(self) -> Self {
        let profiles = Self::all();
        let index = profiles.iter().position(|p| *p == self).unwrap_or(0);
        profiles[(index + 1) % profiles.len()]
    }

    fn prev(self) -> Self {
        let profiles = Self::all();
        let index = profiles.iter().position(|p| *p == self).unwrap_or(0);
        profiles[(index + profiles.len() - 1) % profiles.len()]
    }
}

#[derive(Parser)]
#[command(
    author,
    version,
    about = "Rasterize public fonts into terminal sprites"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Launch interactive browser (default behavior).
    Browser,
    /// Print available catalog fonts.
    List,
    /// Preview a specific font in the terminal.
    Preview(PreviewArgs),
    /// Generate terminal sprite text into file.
    Generate(GenerateArgs),
}

#[derive(Args)]
#[command(group(
    ArgGroup::new("font_source")
        .required(true)
        .args(["font", "font_file"]),
))]
struct PreviewArgs {
    /// Font family name from bundled Google-font catalog.
    #[arg(long)]
    font: Option<String>,
    /// Local TTF file path.
    #[arg(long)]
    font_file: Option<PathBuf>,
    /// Text to preview.
    #[arg(long, default_value = PREVIEW_SAMPLE_TEXT)]
    text: String,
    /// Rasterizer size in px.
    #[arg(long, default_value_t = 24.0)]
    size: f32,
    /// Raster profile: classic|dense|binary|inverted.
    #[arg(long, default_value = "classic")]
    profile: String,
}

#[derive(Args)]
#[command(group(
    ArgGroup::new("font_source")
        .required(true)
        .args(["font", "font_file"]),
))]
struct GenerateArgs {
    /// Font family name from bundled Google-font catalog.
    #[arg(long)]
    font: Option<String>,
    /// Local TTF file path.
    #[arg(long)]
    font_file: Option<PathBuf>,
    /// Text to rasterize.
    #[arg(long)]
    text: String,
    /// Rasterizer size in px.
    #[arg(long, default_value_t = 24.0)]
    size: f32,
    /// Raster profile: classic|dense|binary|inverted.
    #[arg(long, default_value = "classic")]
    profile: String,
    /// Output file path (sprite text).
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogFile {
    fonts: Vec<FontEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct FontEntry {
    family: String,
    variant: String,
    ttf_url: String,
    license: String,
}

#[derive(Debug, Deserialize)]
struct GoogleWebfontsResponse {
    #[serde(default)]
    items: Vec<GoogleWebfontItem>,
}

#[derive(Debug, Deserialize)]
struct GoogleWebfontItem {
    family: String,
    #[serde(default)]
    variants: Vec<String>,
    #[serde(default)]
    files: HashMap<String, String>,
    #[serde(default)]
    license: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let catalog = load_catalog()?;

    match cli.command {
        None | Some(Command::Browser) => run_browser(&catalog.fonts),
        Some(Command::List) => run_list(&catalog.fonts),
        Some(Command::Preview(args)) => run_preview(&catalog.fonts, args),
        Some(Command::Generate(args)) => run_generate(&catalog.fonts, args),
    }
}

fn run_list(fonts: &[FontEntry]) -> Result<()> {
    for font in fonts {
        println!(
            "{} [{}] ({})",
            font.family.as_str().bold(),
            font.variant,
            font.license.as_str()
        );
    }
    io::stdout().flush().context("failed to flush stdout")?;
    Ok(())
}

fn run_preview(fonts: &[FontEntry], args: PreviewArgs) -> Result<()> {
    let profile = parse_profile(&args.profile)?;
    let font_bytes = resolve_font_bytes(fonts, args.font.as_deref(), args.font_file.as_deref())?;
    let art = rasterize_text_to_ascii(&font_bytes, &args.text, args.size, profile)?;
    println!("{art}");
    Ok(())
}

fn run_generate(fonts: &[FontEntry], args: GenerateArgs) -> Result<()> {
    let profile = parse_profile(&args.profile)?;
    let font_bytes = resolve_font_bytes(fonts, args.font.as_deref(), args.font_file.as_deref())?;
    let art = rasterize_text_to_ascii(&font_bytes, &args.text, args.size, profile)?;

    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
    }
    fs::write(&args.output, format!("{art}\n"))
        .with_context(|| format!("failed to write output file {}", args.output.display()))?;

    println!("generated {}", args.output.display());
    Ok(())
}

fn run_browser(fonts: &[FontEntry]) -> Result<()> {
    if fonts.is_empty() {
        bail!("catalog is empty");
    }

    let mut browser_fonts = fonts.to_vec();
    let mut selected = 0usize;
    let mut preview_cache: HashMap<String, String> = HashMap::new();
    let mut message = String::new();
    let mut sample_text = PREVIEW_SAMPLE_TEXT.to_string();
    let mut preview_size = 22.0f32;
    let mut profile = RasterProfile::Classic;
    let mut online_catalog: Option<Vec<FontEntry>> = None;
    let _guard = TerminalGuard::new()?;
    let mut stdout = io::stdout();

    loop {
        ensure_preview(
            &browser_fonts,
            selected,
            &sample_text,
            preview_size,
            profile,
            &mut preview_cache,
            &mut message,
        );
        draw_browser(
            &mut stdout,
            &browser_fonts,
            fonts.len(),
            selected,
            &sample_text,
            preview_size,
            profile,
            &preview_cache,
            &message,
        )?;

        if event::poll(Duration::from_millis(120)).context("failed while polling keyboard")? {
            match event::read().context("failed while reading keyboard event")? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Down | KeyCode::Char('j') => {
                        selected = cmp::min(selected + 1, browser_fonts.len().saturating_sub(1));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyCode::Char('r') => {
                        preview_cache.clear();
                        message = "reloading preview".to_string();
                    }
                    KeyCode::Char('t') => {
                        if let Some(updated) =
                            prompt_sample_text(&mut stdout, sample_text.as_str())?
                        {
                            sample_text = updated;
                            preview_cache.clear();
                            message = "updated preview text".to_string();
                        } else {
                            message = "kept previous preview text".to_string();
                        }
                    }
                    KeyCode::Char('=') | KeyCode::Char('+') => {
                        preview_size = (preview_size + 2.0).min(80.0);
                        preview_cache.clear();
                        message = format!("preview size: {:.0}px", preview_size);
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        preview_size = (preview_size - 2.0).max(8.0);
                        preview_cache.clear();
                        message = format!("preview size: {:.0}px", preview_size);
                    }
                    KeyCode::Char('[') => {
                        profile = profile.prev();
                        preview_cache.clear();
                        message = format!("profile: {}", profile.id());
                    }
                    KeyCode::Char(']') => {
                        profile = profile.next();
                        preview_cache.clear();
                        message = format!("profile: {}", profile.id());
                    }
                    KeyCode::Char('s') => {
                        if let Some(found) =
                            search_font_modal(&mut stdout, fonts, "Local font search")?
                        {
                            select_or_add_font(
                                &found,
                                &mut browser_fonts,
                                &mut selected,
                                &mut message,
                                &mut preview_cache,
                            );
                        } else {
                            message = "search cancelled".to_string();
                        }
                    }
                    KeyCode::Char('S') => {
                        draw_loading_overlay(&mut stdout, "busy loading online fonts...")?;
                        match ensure_online_catalog_loaded(&mut online_catalog) {
                            Ok(catalog) => {
                                if let Some(found) =
                                    search_font_modal(&mut stdout, catalog, "Online font search")?
                                {
                                    select_or_add_font(
                                        &found,
                                        &mut browser_fonts,
                                        &mut selected,
                                        &mut message,
                                        &mut preview_cache,
                                    );
                                } else {
                                    message = "online search cancelled".to_string();
                                }
                            }
                            Err(error) => {
                                message = format!("online fonts unavailable: {error}");
                            }
                        }
                    }
                    KeyCode::Char('C') => {
                        let entry = &browser_fonts[selected];
                        let command =
                            build_generate_command(entry, &sample_text, preview_size, profile);
                        copy_text_osc52(&command, &mut stdout)?;
                        message = "copied generate command to clipboard".to_string();
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    Ok(())
}

fn draw_browser(
    stdout: &mut io::Stdout,
    fonts: &[FontEntry],
    catalog_total: usize,
    selected: usize,
    sample_text: &str,
    preview_size: f32,
    profile: RasterProfile,
    preview_cache: &HashMap<String, String>,
    message: &str,
) -> Result<()> {
    let (w, h) = terminal::size().context("failed to read terminal size")?;
    let left_width = cmp::min(38u16, w.saturating_sub(20));
    let list_height = h.saturating_sub(6) as usize;
    let mut start = selected.saturating_sub(list_height / 2);
    if start + list_height > fonts.len() {
        start = fonts.len().saturating_sub(list_height);
    }
    let end = cmp::min(start + list_height, fonts.len());

    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
    queue!(
        stdout,
        MoveTo(0, 0),
        Print("TTF Rasterizer Browser".bold()),
        MoveTo(0, 1),
        Print(
            "↑/↓ j/k: nav   s: local search   S: online search   C: copy generate cmd   t: text   -/=: size   [ ]: profile   r: reload   q: quit"
        ),
        MoveTo(0, 2),
        Print(format!(
            "fonts: {}/{}   size: {:.0}px   profile: {}   sample: {}",
            fonts.len(),
            catalog_total,
            preview_size,
            profile.id(),
            single_line_sample(sample_text, 36)
        ))
    )?;

    let mut row = 4u16;
    for index in start..end {
        let marker = if index == selected { ">" } else { " " };
        let item = format!("{marker} {}", fonts[index].family);
        queue!(
            stdout,
            MoveTo(0, row),
            Print(truncate_to_width(&item, left_width as usize).to_string())
        )?;
        row += 1;
    }

    let selected_font = &fonts[selected];
    let preview_x = left_width + 2;
    queue!(
        stdout,
        MoveTo(preview_x, 0),
        Print(selected_font.family.as_str().bold()),
        MoveTo(preview_x, 1),
        Print(format!(
            "variant: {}  license: {}",
            selected_font.variant, selected_font.license
        )),
        MoveTo(preview_x, 2),
        Print("preview:"),
    )?;

    let preview_key = format!(
        "{}|{:.1}|{}|{}",
        font_id(selected_font),
        preview_size,
        profile.id(),
        sample_text
    );
    let preview_text = preview_cache
        .get(&preview_key)
        .map(|s| s.as_str())
        .unwrap_or("loading preview...");
    for (i, line) in preview_text.lines().enumerate() {
        let y = 3u16 + i as u16;
        if y >= h.saturating_sub(1) {
            break;
        }
        queue!(
            stdout,
            MoveTo(preview_x, y),
            Print(truncate_to_width(
                line,
                w.saturating_sub(preview_x) as usize
            ))
        )?;
    }

    if !message.is_empty() {
        queue!(
            stdout,
            MoveTo(0, h.saturating_sub(1)),
            Print(truncate_to_width(message, w as usize))
        )?;
    }

    stdout.flush().context("failed to flush browser frame")
}

fn ensure_preview(
    fonts: &[FontEntry],
    index: usize,
    sample_text: &str,
    preview_size: f32,
    profile: RasterProfile,
    cache: &mut HashMap<String, String>,
    message: &mut String,
) {
    let entry = &fonts[index];
    let cache_key = format!(
        "{}|{:.1}|{}|{}",
        font_id(entry),
        preview_size,
        profile.id(),
        sample_text
    );
    if cache.contains_key(&cache_key) {
        return;
    }

    match load_font_bytes(entry)
        .and_then(|bytes| rasterize_text_to_ascii(&bytes, sample_text, preview_size, profile))
    {
        Ok(preview) => {
            cache.insert(cache_key, preview);
            *message = format!(
                "loaded {} @ {:.0}px ({})",
                entry.family,
                preview_size,
                profile.id()
            );
        }
        Err(error) => {
            let err = format!("preview error: {error}");
            cache.insert(cache_key, err.clone());
            *message = format!("{} ({})", entry.family, error);
        }
    }
}

fn search_font_modal(
    stdout: &mut io::Stdout,
    catalog: &[FontEntry],
    title: &str,
) -> Result<Option<FontEntry>> {
    let mut query = String::new();
    let mut selected = 0usize;

    loop {
        let filtered = filter_fonts(catalog, &query);
        if selected >= filtered.len() {
            selected = filtered.len().saturating_sub(1);
        }

        draw_search_modal(stdout, title, catalog, &query, &filtered, selected)?;

        if event::poll(Duration::from_millis(120)).context("failed while polling search input")? {
            match event::read().context("failed while reading search input")? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Esc => return Ok(None),
                    KeyCode::Enter => {
                        if let Some(idx) = filtered.get(selected) {
                            return Ok(Some(catalog[*idx].clone()));
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        selected = cmp::min(selected + 1, filtered.len().saturating_sub(1));
                    }
                    KeyCode::PageUp => {
                        let page = search_page_step();
                        selected = selected.saturating_sub(page);
                    }
                    KeyCode::PageDown => {
                        let page = search_page_step();
                        selected = cmp::min(selected + page, filtered.len().saturating_sub(1));
                    }
                    KeyCode::Backspace => {
                        query.pop();
                    }
                    KeyCode::Char(ch) if !ch.is_control() => {
                        query.push(ch);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn draw_search_modal(
    stdout: &mut io::Stdout,
    title: &str,
    catalog: &[FontEntry],
    query: &str,
    filtered: &[usize],
    selected: usize,
) -> Result<()> {
    let (_, h) = terminal::size().context("failed to read terminal size")?;
    let max_rows = h.saturating_sub(4) as usize;
    let mut start = selected.saturating_sub(max_rows / 2);
    if start + max_rows > filtered.len() {
        start = filtered.len().saturating_sub(max_rows);
    }
    let end = cmp::min(start + max_rows, filtered.len());

    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
    queue!(
        stdout,
        MoveTo(0, 0),
        Print(title.bold()),
        MoveTo(0, 1),
        Print("Type to filter • ↑/↓/PgUp/PgDn or j/k • Enter add/select • Esc cancel"),
        MoveTo(0, 2),
        Print(format!("query: {}", query))
    )?;

    if filtered.is_empty() {
        queue!(stdout, MoveTo(0, 4), Print("No results"))?;
    } else {
        let mut row = 4u16;
        for idx in &filtered[start..end] {
            let entry = &catalog[*idx];
            let marker = if filtered[selected] == *idx { ">" } else { " " };
            queue!(
                stdout,
                MoveTo(0, row),
                Print(format!(
                    "{} {} [{}] ({})",
                    marker, entry.family, entry.variant, entry.license
                ))
            )?;
            row += 1;
        }
    }

    stdout.flush().context("failed to flush search modal")
}

fn filter_fonts(catalog: &[FontEntry], query: &str) -> Vec<usize> {
    let normalized = query.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return (0..catalog.len()).collect();
    }

    catalog
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let family = entry.family.to_ascii_lowercase();
            let variant = entry.variant.to_ascii_lowercase();
            if family.contains(&normalized) || variant.contains(&normalized) {
                Some(index)
            } else {
                None
            }
        })
        .collect()
}

fn search_page_step() -> usize {
    terminal::size()
        .map(|(_, h)| h.saturating_sub(6).max(1) as usize)
        .unwrap_or(10)
}

fn same_font(a: &FontEntry, b: &FontEntry) -> bool {
    a.family.eq_ignore_ascii_case(&b.family) && a.variant.eq_ignore_ascii_case(&b.variant)
}

fn font_id(entry: &FontEntry) -> String {
    format!("{}::{}", entry.family, entry.variant)
}

fn build_generate_command(
    entry: &FontEntry,
    sample_text: &str,
    preview_size: f32,
    profile: RasterProfile,
) -> String {
    let output = format!(
        "mods/base/assets/fonts/{}-{}-{}.txt",
        slugify(&entry.family),
        slugify(&entry.variant),
        profile.id()
    );
    format!(
        "cargo run -p ttf-rasterizer -- generate --font {} --text {} --profile {} --size {:.0} --output {}",
        shell_quote(&entry.family),
        shell_quote(sample_text),
        profile.id(),
        preview_size,
        shell_quote(&output),
    )
}

fn copy_text_osc52(text: &str, stdout: &mut io::Stdout) -> Result<()> {
    let encoded = BASE64_STANDARD.encode(text.as_bytes());
    write!(stdout, "\x1b]52;c;{}\x07", encoded)
        .context("failed to emit OSC52 clipboard sequence")?;
    stdout
        .flush()
        .context("failed to flush OSC52 clipboard sequence")?;
    Ok(())
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{}'", escaped)
}

fn select_or_add_font(
    found: &FontEntry,
    browser_fonts: &mut Vec<FontEntry>,
    selected: &mut usize,
    message: &mut String,
    preview_cache: &mut HashMap<String, String>,
) {
    if let Some(index) = browser_fonts
        .iter()
        .position(|entry| same_font(entry, found))
    {
        *selected = index;
        *message = format!("selected {}", found.family);
    } else {
        browser_fonts.push(found.clone());
        *selected = browser_fonts.len().saturating_sub(1);
        *message = format!("added {}", found.family);
    }
    preview_cache.clear();
}

fn draw_loading_overlay(stdout: &mut io::Stdout, message: &str) -> Result<()> {
    queue!(
        stdout,
        MoveTo(0, 0),
        Clear(ClearType::All),
        MoveTo(0, 0),
        Print("TTF Rasterizer Browser".bold()),
        MoveTo(0, 2),
        Print(message),
        MoveTo(0, 3),
        Print("please wait...")
    )?;
    stdout.flush().context("failed to draw loading overlay")
}

fn ensure_online_catalog_loaded(cache: &mut Option<Vec<FontEntry>>) -> Result<&[FontEntry]> {
    if cache.is_none() {
        let fetched = fetch_online_google_fonts()?;
        if fetched.is_empty() {
            bail!("online catalog is empty");
        }
        *cache = Some(fetched);
    }
    Ok(cache.as_deref().unwrap_or(&[]))
}

fn prompt_sample_text(stdout: &mut io::Stdout, current: &str) -> Result<Option<String>> {
    let (_, h) = terminal::size().context("failed to read terminal size")?;
    let current_line = single_line_sample(current, 64);
    disable_raw_mode().context("failed to leave raw mode for prompt input")?;
    execute!(stdout, Show).context("failed to show cursor for prompt input")?;

    queue!(
        stdout,
        MoveTo(0, h.saturating_sub(1)),
        Clear(ClearType::CurrentLine),
        Print(format!(
            "Preview text (single line, blank keeps current) [{}]: ",
            current_line
        ))
    )?;
    stdout.flush().context("failed to flush prompt line")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read prompt input")?;

    execute!(stdout, Hide).context("failed to hide cursor after prompt input")?;
    enable_raw_mode().context("failed to re-enable raw mode after prompt input")?;

    let value = input.trim().to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn parse_profile(value: &str) -> Result<RasterProfile> {
    let normalized = value.trim().to_ascii_lowercase();
    RasterProfile::from_id(&normalized).ok_or_else(|| {
        anyhow!(
            "unknown profile '{value}', expected one of: {}",
            RasterProfile::all()
                .iter()
                .map(|p| p.id())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

fn resolve_font_bytes(
    fonts: &[FontEntry],
    family: Option<&str>,
    font_file: Option<&Path>,
) -> Result<Vec<u8>> {
    match (family, font_file) {
        (Some(name), None) => {
            let entry = find_font(fonts, name)
                .ok_or_else(|| anyhow!("font '{name}' not found in catalog"))?;
            load_font_bytes(entry)
        }
        (None, Some(path)) => fs::read(path)
            .with_context(|| format!("failed to read local font file {}", path.display())),
        _ => bail!("provide exactly one font source"),
    }
}

fn find_font<'a>(fonts: &'a [FontEntry], family: &str) -> Option<&'a FontEntry> {
    fonts
        .iter()
        .find(|entry| entry.family.eq_ignore_ascii_case(family))
}

fn load_font_bytes(entry: &FontEntry) -> Result<Vec<u8>> {
    let cache_file = cache_file_path(entry);
    if cache_file.exists() {
        return fs::read(&cache_file)
            .with_context(|| format!("failed to read cached font {}", cache_file.display()));
    }

    let cache_dir = cache_file
        .parent()
        .ok_or_else(|| anyhow!("failed to determine cache directory"))?;
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("failed to create cache directory {}", cache_dir.display()))?;

    let client = Client::new();
    let response = client
        .get(&entry.ttf_url)
        .header(USER_AGENT, "shell-quest-ttf-rasterizer/0.1")
        .send()
        .and_then(|res| res.error_for_status())
        .with_context(|| format!("failed to download font from {}", entry.ttf_url))?;
    let bytes = response
        .bytes()
        .context("failed to read downloaded bytes")?;
    fs::write(&cache_file, &bytes)
        .with_context(|| format!("failed to cache font at {}", cache_file.display()))?;
    Ok(bytes.to_vec())
}

fn cache_file_path(entry: &FontEntry) -> PathBuf {
    cache_root().join("fonts").join(format!(
        "{}-{}.ttf",
        slugify(&entry.family),
        slugify(&entry.variant)
    ))
}

fn cache_root() -> PathBuf {
    if let Ok(custom) = std::env::var("SHELL_QUEST_FONT_CACHE") {
        return PathBuf::from(custom);
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(xdg)
            .join("shell-quest")
            .join("ttf-rasterizer");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".cache")
            .join("shell-quest")
            .join("ttf-rasterizer");
    }
    PathBuf::from(".cache")
        .join("shell-quest")
        .join("ttf-rasterizer")
}

fn load_catalog() -> Result<CatalogFile> {
    let raw = include_str!("../catalog/google_fonts.yaml");
    let parsed: CatalogFile =
        serde_yaml::from_str(raw).context("failed to parse bundled google font catalog")?;
    if parsed.fonts.is_empty() {
        bail!("bundled catalog is empty");
    }
    Ok(parsed)
}

fn fetch_online_google_fonts() -> Result<Vec<FontEntry>> {
    let api_key = resolve_google_fonts_api_key()?;

    let url =
        format!("https://www.googleapis.com/webfonts/v1/webfonts?sort=popularity&key={api_key}");
    let client = Client::new();
    let raw = client
        .get(url)
        .header(USER_AGENT, "shell-quest-ttf-rasterizer/0.1")
        .send()
        .and_then(|res| res.error_for_status())
        .context("failed to fetch online font catalog")?
        .text()
        .context("failed to read online font catalog payload")?;

    let parsed: GoogleWebfontsResponse =
        serde_json::from_str(&raw).context("failed to parse online font catalog JSON")?;

    let mut fonts = Vec::new();
    let mut seen = HashSet::new();
    for item in parsed.items {
        let license = if item.license.trim().is_empty() {
            "Unknown".to_string()
        } else {
            item.license.clone()
        };

        let variant = if item.variants.iter().any(|v| v == "regular") {
            "regular".to_string()
        } else {
            item.variants
                .first()
                .cloned()
                .unwrap_or_else(|| "regular".to_string())
        };

        if let Some(url) = item.files.get(&variant) {
            let id = format!(
                "{}::{}",
                item.family.to_ascii_lowercase(),
                variant.to_ascii_lowercase()
            );
            if seen.insert(id) {
                fonts.push(FontEntry {
                    family: item.family.clone(),
                    variant: variant.clone(),
                    ttf_url: url.replacen("http://", "https://", 1),
                    license: license.clone(),
                });
            }
        }
    }

    fonts.sort_by(|a, b| a.family.cmp(&b.family).then(a.variant.cmp(&b.variant)));
    Ok(fonts)
}

fn resolve_google_fonts_api_key() -> Result<String> {
    for env_name in ["GOOGLE_FONTS_API_KEY", "SHELL_QUEST_GOOGLE_FONTS_API_KEY"] {
        if let Ok(value) = std::env::var(env_name) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }

    for path in google_fonts_env_file_candidates() {
        if !path.exists() {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read env file {}", path.display()))?;

        for line in raw.lines() {
            if let Some(value) = parse_export_var(line, "GOOGLE_FONTS_API_KEY")
                .or_else(|| parse_export_var(line, "SHELL_QUEST_GOOGLE_FONTS_API_KEY"))
            {
                return Ok(value);
            }
        }
    }

    bail!(
        "GOOGLE_FONTS_API_KEY not available. Set env var or add it to ~/.config/shell-quest/env.sh"
    )
}

fn google_fonts_env_file_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
        paths.push(
            PathBuf::from(xdg_config_home)
                .join("shell-quest")
                .join("env.sh"),
        );
    }
    if let Ok(home) = std::env::var("HOME") {
        paths.push(
            PathBuf::from(home)
                .join(".config")
                .join("shell-quest")
                .join("env.sh"),
        );
    }
    paths
}

fn parse_export_var(line: &str, key: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }

    let without_export = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let (name, value) = without_export.split_once('=')?;
    if name.trim() != key {
        return None;
    }

    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let unquoted = if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        value[1..value.len().saturating_sub(1)].trim()
    } else {
        value
    };

    if unquoted.is_empty() {
        None
    } else {
        Some(unquoted.to_string())
    }
}

fn rasterize_text_to_ascii(
    font_bytes: &[u8],
    text: &str,
    size: f32,
    profile: RasterProfile,
) -> Result<String> {
    let font = Font::from_bytes(font_bytes, FontSettings::default())
        .map_err(|error| anyhow!("failed to parse font bytes: {error}"))?;
    let mut lines = Vec::new();
    for (line_idx, line) in text.lines().enumerate() {
        let rendered = rasterize_line(&font, line, size, profile);
        lines.extend(rendered);
        if line_idx + 1 < text.lines().count() {
            lines.push(String::new());
        }
    }

    Ok(trim_vertical(lines).join("\n"))
}

fn rasterize_line(font: &Font, text: &str, size: f32, profile: RasterProfile) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    struct Glyph {
        width: usize,
        height: usize,
        x: usize,
        bitmap: Vec<u8>,
    }

    let mut glyphs = Vec::new();
    let mut max_height = 1usize;
    let mut pen_x = 0usize;

    for ch in text.chars() {
        if ch == ' ' {
            pen_x += (size * 0.35).ceil() as usize + 1;
            continue;
        }

        let (metrics, bitmap) = font.rasterize(ch, size);
        let advance = cmp::max((metrics.advance_width.ceil() as usize).saturating_add(1), 1);
        if metrics.width > 0 && metrics.height > 0 {
            max_height = cmp::max(max_height, metrics.height);
            glyphs.push(Glyph {
                width: metrics.width,
                height: metrics.height,
                x: pen_x,
                bitmap,
            });
        }
        pen_x += cmp::max(advance, metrics.width.saturating_add(1));
    }

    if pen_x == 0 {
        pen_x = 1;
    }
    let mut canvas = vec![0u8; max_height * pen_x];
    for glyph in glyphs {
        let y_offset = (max_height.saturating_sub(glyph.height)) / 2;
        for row in 0..glyph.height {
            for col in 0..glyph.width {
                let src_idx = row * glyph.width + col;
                let dst_x = glyph.x + col;
                if dst_x >= pen_x {
                    continue;
                }
                let dst_y = y_offset + row;
                let dst_idx = dst_y * pen_x + dst_x;
                canvas[dst_idx] = cmp::max(canvas[dst_idx], glyph.bitmap[src_idx]);
            }
        }
    }

    let ramp: Vec<char> = profile.ramp().chars().collect();
    let mut lines = Vec::with_capacity(max_height);
    for y in 0..max_height {
        let mut line = String::with_capacity(pen_x);
        for x in 0..pen_x {
            let p = canvas[y * pen_x + x] as usize;
            let idx = p * (ramp.len().saturating_sub(1)) / 255;
            line.push(ramp[idx]);
        }
        while line.ends_with(' ') {
            line.pop();
        }
        lines.push(line);
    }

    trim_vertical(lines)
}

fn trim_vertical(lines: Vec<String>) -> Vec<String> {
    let first = lines.iter().position(|line| !line.trim().is_empty());
    let last = lines.iter().rposition(|line| !line.trim().is_empty());
    match (first, last) {
        (Some(start), Some(end)) => lines[start..=end].to_vec(),
        _ => vec![String::new()],
    }
}

fn truncate_to_width(text: &str, width: usize) -> &str {
    if width == 0 {
        return "";
    }
    if text.chars().count() <= width {
        return text;
    }

    let mut idx = text.len();
    for (count, (byte_index, _)) in text.char_indices().enumerate() {
        if count == width {
            idx = byte_index;
            break;
        }
    }
    &text[..idx]
}

fn single_line_sample(text: &str, width: usize) -> String {
    let first = text.lines().next().unwrap_or_default();
    truncate_to_width(first, width).to_string()
}

fn slugify(input: &str) -> String {
    let mut slug = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

struct TerminalGuard;

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw terminal mode")?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)
            .context("failed to enter alternate screen")?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_generate_command, filter_fonts, parse_export_var, parse_profile, shell_quote,
        slugify, truncate_to_width, FontEntry, RasterProfile,
    };

    #[test]
    fn slugify_makes_safe_names() {
        assert_eq!(slugify("JetBrains Mono"), "jetbrains-mono");
        assert_eq!(slugify("Noto+Sans"), "noto-sans");
    }

    #[test]
    fn truncate_keeps_char_boundaries() {
        assert_eq!(truncate_to_width("abcdef", 3), "abc");
        assert_eq!(truncate_to_width("abc", 10), "abc");
    }

    #[test]
    fn parses_raster_profiles() {
        assert_eq!(
            parse_profile("classic").expect("classic profile should parse"),
            RasterProfile::Classic
        );
        assert_eq!(
            parse_profile("dense").expect("dense profile should parse"),
            RasterProfile::Dense
        );
        assert!(parse_profile("unknown").is_err());
    }

    #[test]
    fn filters_fonts_case_insensitively() {
        let catalog = vec![
            FontEntry {
                family: "JetBrains Mono".to_string(),
                variant: "regular".to_string(),
                ttf_url: "u1".to_string(),
                license: "OFL".to_string(),
            },
            FontEntry {
                family: "Space Mono".to_string(),
                variant: "bold".to_string(),
                ttf_url: "u2".to_string(),
                license: "OFL".to_string(),
            },
        ];

        assert_eq!(filter_fonts(&catalog, "jet"), vec![0]);
        assert_eq!(filter_fonts(&catalog, "BOLD"), vec![1]);
        assert_eq!(filter_fonts(&catalog, ""), vec![0, 1]);
    }

    #[test]
    fn parses_exported_key_lines() {
        assert_eq!(
            parse_export_var(
                "export GOOGLE_FONTS_API_KEY=\"abc123\"",
                "GOOGLE_FONTS_API_KEY"
            ),
            Some("abc123".to_string())
        );
        assert_eq!(
            parse_export_var(
                "SHELL_QUEST_GOOGLE_FONTS_API_KEY=zzz",
                "SHELL_QUEST_GOOGLE_FONTS_API_KEY"
            ),
            Some("zzz".to_string())
        );
        assert_eq!(
            parse_export_var("# export GOOGLE_FONTS_API_KEY=none", "GOOGLE_FONTS_API_KEY"),
            None
        );
    }

    #[test]
    fn builds_generate_command_with_quoted_fields() {
        let entry = FontEntry {
            family: "JetBrains Mono".to_string(),
            variant: "regular".to_string(),
            ttf_url: "u".to_string(),
            license: "OFL".to_string(),
        };
        let command = build_generate_command(&entry, "SHELL QUEST", 24.0, RasterProfile::Dense);
        assert!(command.contains("--font 'JetBrains Mono'"));
        assert!(command.contains("--text 'SHELL QUEST'"));
        assert!(command.contains("--profile dense"));
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }
}
