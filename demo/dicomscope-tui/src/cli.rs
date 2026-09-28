//! The command line: every flag, argument and subcommand, as `clap` derive
//! types. The doc comments here are the `--help` text and this page alike,
//! so the two cannot drift apart.
//!
//! Without a subcommand `dicomscope-tui` opens the interactive viewer on
//! the paths it is given; see [`ViewArgs`]. Every subcommand reads the
//! same inputs, prints plain text by default or JSON with `--json`, and
//! never touches the terminal's screen, so it can run in scripts and CI.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// dicomscope in the terminal: DICOM studies rendered by wgpu (Metal,
/// Vulkan, DX12 or GL) and shown inline, next to the HL7 v2 order they
/// belong to.
///
/// Run without a subcommand to open the interactive viewer. Paths may be
/// DICOM files, folders or zip archives (one study) and `.hl7`/`.txt` order
/// messages, in any mix.
#[derive(Debug, Parser)]
#[command(
    name = "dicomscope-tui",
    version,
    args_conflicts_with_subcommands = true,
    after_long_help = AFTER_HELP,
)]
pub struct Cli {
    /// A subcommand; none opens the viewer.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// The viewer's arguments when no subcommand is given.
    #[command(flatten)]
    pub view: ViewArgs,

    /// GPU options for every command.
    #[command(flatten)]
    pub gpu: GpuArgs,
}

const AFTER_HELP: &str = "\
Examples:
  dicomscope-tui study.zip order.hl7          open the viewer
  dicomscope-tui info ./CT-head               series and documents
  dicomscope-tui tags scan.dcm --grep Patient tags matching a pattern
  dicomscope-tui render study.zip -o out.png  one slice through the GPU
  dicomscope-tui render study.zip --inline    ...straight into the terminal
  dicomscope-tui link study.zip order.hl7     does the order match? (exit 0/3)
  dicomscope-tui fhir study.zip order.hl7     FHIR R4 transaction Bundle
  dicomscope-tui worklist order.hl7 -o mwl.dcm
  dicomscope-tui gpu                          adapters wgpu can use

Exit status: 0 success, 1 error, 2 usage, 3 study and order do not link cleanly.";

/// The non-interactive commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the interactive viewer (the default without a subcommand).
    View(ViewArgs),
    /// The study: patient, identifiers, series, documents, skipped files.
    Info(InfoArgs),
    /// The DICOM header of one slice, nested sequences indented.
    Tags(TagsArgs),
    /// Render one slice on the GPU to a PNG file or into the terminal.
    Render(RenderArgs),
    /// Parse an HL7 v2 order: message type, identifiers, warnings.
    Order(OrderArgs),
    /// Check a study against an order: identifiers, linkage path, chain.
    ///
    /// Exits with status 3 when the study and the order do not link, a key
    /// conflicts, or they link but the patient IDs differ, so scripts can
    /// branch on it.
    Link(LinkArgs),
    /// FHIR R4 resources for a study and its order.
    Fhir(FhirArgs),
    /// The Modality Worklist item an order becomes (DICOM Part 10).
    Worklist(WorklistArgs),
    /// Structured Reports and encapsulated PDFs in a study.
    Docs(DocsArgs),
    /// The GPU adapters wgpu finds, per backend.
    Gpu,
}

/// Paths and start position for the viewer.
#[derive(Debug, Args, Default, Clone)]
pub struct ViewArgs {
    /// Study files, folders or zips, and `.hl7`/`.txt` orders.
    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    /// An order message to open, whatever its extension.
    #[arg(long, value_name = "FILE")]
    pub order: Option<PathBuf>,

    /// Series to start on, counted from 1 in the listed order.
    #[arg(long, value_name = "N", default_value_t = 1)]
    pub series: usize,

    /// Slice to start on within the series, counted from 1.
    #[arg(long, value_name = "N", default_value_t = 1)]
    pub slice: usize,

    /// How images reach the terminal. `auto` asks the terminal what it
    /// supports: kitty graphics (kitty, Ghostty, WezTerm), iTerm2 inline
    /// images (iTerm2, WezTerm), sixel (foot, xterm -ti vt340, mlterm),
    /// and falls back to Unicode half blocks, which work everywhere.
    #[arg(long, value_enum, value_name = "PROTOCOL", default_value_t = Protocol::Auto)]
    pub protocol: Protocol,

    /// Open the viewer in its own window instead of the terminal: the same
    /// panels and keys, drawn by wgpu, with the image composited at the
    /// window's full resolution. Needs a desktop session, not SSH.
    #[arg(long, short = 'w')]
    pub window: bool,

    /// A TrueType or OpenType font (`.ttf`, `.otf`, first face of a
    /// `.ttc`) for `--window`. Defaults to the bundled Cascadia Mono.
    #[arg(long, value_name = "FILE", requires = "window")]
    pub font: Option<PathBuf>,
}

/// GPU selection, shared by the viewer and `render`.
#[derive(Debug, Args, Clone, Copy)]
pub struct GpuArgs {
    /// The wgpu backend that renders images. `auto` prefers Metal on
    /// macOS, Vulkan or DX12 elsewhere, and GL last.
    #[arg(
        long,
        global = true,
        value_enum,
        value_name = "BACKEND",
        default_value_t = Backend::Auto
    )]
    pub backend: Backend,
}

/// A wgpu backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum Backend {
    /// The best the platform offers.
    #[default]
    Auto,
    /// Apple Metal (macOS, iOS).
    Metal,
    /// Vulkan (Linux, Windows, Android; MoltenVK on macOS if installed).
    Vulkan,
    /// Direct3D 12 (Windows).
    Dx12,
    /// OpenGL / OpenGL ES.
    Gl,
}

impl Backend {
    /// The wgpu backend set this selects.
    pub fn backends(self) -> wgpu::Backends {
        match self {
            Backend::Auto => wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            Backend::Metal => wgpu::Backends::METAL,
            Backend::Vulkan => wgpu::Backends::VULKAN,
            Backend::Dx12 => wgpu::Backends::DX12,
            Backend::Gl => wgpu::Backends::GL,
        }
    }
}

/// A terminal graphics protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum Protocol {
    /// Ask the terminal.
    #[default]
    Auto,
    /// The kitty graphics protocol.
    Kitty,
    /// iTerm2 inline images.
    Iterm2,
    /// DEC sixel graphics.
    Sixel,
    /// Unicode half blocks with 24-bit colour; no graphics support needed.
    Halfblocks,
}

/// Output format for the text commands.
#[derive(Debug, Args, Clone, Copy, Default)]
pub struct Format {
    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,
}

/// Arguments of `info`.
#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Study files, folders or zips, read as one study.
    #[arg(value_name = "STUDY", required = true)]
    pub study: Vec<PathBuf>,
    /// Text or JSON.
    #[command(flatten)]
    pub format: Format,
}

/// Which slice a command works on.
#[derive(Debug, Args, Clone, Copy)]
pub struct SliceArgs {
    /// Series, counted from 1 in the order `info` lists them.
    #[arg(long, value_name = "N", default_value_t = 1)]
    pub series: usize,
    /// Slice within the series, counted from 1.
    #[arg(long, value_name = "N", default_value_t = 1)]
    pub slice: usize,
}

/// Arguments of `tags`.
#[derive(Debug, Args)]
pub struct TagsArgs {
    /// Study files, folders or zips.
    #[arg(value_name = "STUDY", required = true)]
    pub study: Vec<PathBuf>,
    /// The slice.
    #[command(flatten)]
    pub at: SliceArgs,
    /// Only rows whose tag, keyword, VR or value contains this text
    /// (case-insensitive).
    #[arg(long, short = 'g', value_name = "TEXT")]
    pub grep: Option<String>,
    /// Text or JSON.
    #[command(flatten)]
    pub format: Format,
}

/// Arguments of `render`.
#[derive(Debug, Args)]
pub struct RenderArgs {
    /// Study files, folders or zips.
    #[arg(value_name = "STUDY", required = true)]
    pub study: Vec<PathBuf>,
    /// The slice to render.
    #[command(flatten)]
    pub at: SliceArgs,
    /// PNG file to write.
    #[arg(
        long,
        short = 'o',
        value_name = "FILE.png",
        required_unless_present = "inline"
    )]
    pub output: Option<PathBuf>,
    /// Show the image in this terminal instead of (or as well as) writing
    /// a file. Uses `--protocol` to pick how.
    #[arg(long)]
    pub inline: bool,
    /// Protocol for `--inline`.
    #[arg(long, value_enum, value_name = "PROTOCOL", default_value_t = Protocol::Auto)]
    pub protocol: Protocol,
    /// Output size in pixels, `WIDTHxHEIGHT`; the image is fitted inside.
    /// Defaults to the image's own size (after rotation).
    #[arg(long, value_name = "WxH", value_parser = parse_size)]
    pub size: Option<(u32, u32)>,
    /// Window centre and width, `CENTER/WIDTH` (e.g. `40/400` for soft
    /// tissue). Defaults to the file's first window, else the value range.
    #[arg(long, value_name = "C/W", value_parser = parse_window, allow_hyphen_values = true)]
    pub window: Option<(f32, f32)>,
    /// Zoom factor on top of fitting, about the centre.
    #[arg(long, value_name = "F", default_value_t = 1.0)]
    pub zoom: f32,
    /// Quarter turns clockwise, 0 to 3.
    #[arg(long, value_name = "N", default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=3))]
    pub rotate: u8,
    /// Mirror left-right.
    #[arg(long)]
    pub flip_h: bool,
    /// Mirror top-bottom.
    #[arg(long)]
    pub flip_v: bool,
    /// Invert greyscale.
    #[arg(long)]
    pub invert: bool,
    /// Local contrast enhancement (greyscale), as in the viewer's `e`.
    #[arg(long)]
    pub enhance: bool,
    /// Nearest-neighbour instead of bilinear when magnifying.
    #[arg(long)]
    pub sharp: bool,
}

/// Arguments of `order`.
#[derive(Debug, Args)]
pub struct OrderArgs {
    /// The HL7 v2 message (ORM^O01, OMI^O23, ORU^R01 ...). UTF-8 or
    /// Latin-1; segment separators CR, LF or CRLF.
    #[arg(value_name = "ORDER")]
    pub order: PathBuf,
    /// Also print the raw message.
    #[arg(long)]
    pub raw: bool,
    /// Text or JSON.
    #[command(flatten)]
    pub format: Format,
}

/// A study and the order to check it against.
#[derive(Debug, Args)]
pub struct StudyAndOrder {
    /// Study paths, then the order as the last argument (or use `--order`).
    #[arg(value_name = "PATH", required = true)]
    pub paths: Vec<PathBuf>,
    /// The order message, if it is not the last path.
    #[arg(long, value_name = "FILE")]
    pub order: Option<PathBuf>,
}

impl StudyAndOrder {
    /// (study paths, order path): `--order`, else the last `.hl7`/`.txt`
    /// path, else the last path.
    pub fn split(&self) -> Result<(Vec<PathBuf>, PathBuf), String> {
        let mut paths = self.paths.clone();
        let order = match &self.order {
            Some(o) => o.clone(),
            None => {
                let i = paths
                    .iter()
                    .rposition(|p| is_order_path(p))
                    .unwrap_or(paths.len().saturating_sub(1));
                paths.remove(i)
            }
        };
        if paths.is_empty() {
            return Err("no study path given before the order".into());
        }
        Ok((paths, order))
    }
}

/// Arguments of `link`.
#[derive(Debug, Args)]
pub struct LinkArgs {
    /// The study and the order.
    #[command(flatten)]
    pub inputs: StudyAndOrder,
    /// Text or JSON.
    #[command(flatten)]
    pub format: Format,
}

/// Arguments of `fhir`.
#[derive(Debug, Args)]
pub struct FhirArgs {
    /// The study and the order.
    #[command(flatten)]
    pub inputs: StudyAndOrder,
    /// Which resource to print.
    #[arg(long, short = 'r', value_enum, default_value_t = FhirResource::Bundle)]
    pub resource: FhirResource,
    /// Write to this file instead of standard output.
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,
}

/// A FHIR resource `fhir` can print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FhirResource {
    /// A transaction Bundle with all three resources.
    Bundle,
    /// Patient, from PID.
    Patient,
    /// ServiceRequest, from ORC/OBR/IPC.
    ServiceRequest,
    /// ImagingStudy, from the DICOM study.
    ImagingStudy,
}

/// Arguments of `worklist`.
#[derive(Debug, Args)]
pub struct WorklistArgs {
    /// The HL7 v2 order message.
    #[arg(value_name = "ORDER")]
    pub order: PathBuf,
    /// Write the worklist item as a DICOM Part 10 file.
    #[arg(long, short = 'o', value_name = "FILE.dcm")]
    pub output: Option<PathBuf>,
    /// Text or JSON.
    #[command(flatten)]
    pub format: Format,
}

/// Arguments of `docs`.
#[derive(Debug, Args)]
pub struct DocsArgs {
    /// Study files, folders or zips.
    #[arg(value_name = "STUDY", required = true)]
    pub study: Vec<PathBuf>,
    /// Print (report) or save (PDF, needs `-o`) document N, counted from 1.
    #[arg(long, value_name = "N")]
    pub show: Option<usize>,
    /// Where `--show` writes a PDF, or a report's text.
    #[arg(long, short = 'o', value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// Text or JSON.
    #[command(flatten)]
    pub format: Format,
}

/// `.hl7` or `.txt`: an order rather than a study, as the viewer decides.
pub fn is_order_path(p: &std::path::Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("hl7") || e.eq_ignore_ascii_case("txt"))
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("expected WIDTHxHEIGHT, got {s:?}"))?;
    let n = |v: &str| {
        v.trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=16384).contains(n))
            .ok_or_else(|| format!("{v:?} is not a size from 1 to 16384"))
    };
    Ok((n(w)?, n(h)?))
}

fn parse_window(s: &str) -> Result<(f32, f32), String> {
    let (c, w) = s
        .split_once('/')
        .ok_or_else(|| format!("expected CENTER/WIDTH, got {s:?}"))?;
    let n = |v: &str| {
        v.trim()
            .parse::<f32>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("{v:?} is not a number"))
    };
    let (c, w) = (n(c)?, n(w)?);
    if w < 1.0 {
        return Err("window width must be at least 1".into());
    }
    Ok((c, w))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_line_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn sizes_and_windows_parse() {
        assert_eq!(parse_size("800x600"), Ok((800, 600)));
        assert!(parse_size("0x600").is_err());
        assert!(parse_size("800").is_err());
        assert_eq!(parse_window("-600/1500"), Ok((-600.0, 1500.0)));
        assert!(parse_window("40/0").is_err());
    }

    #[test]
    fn bare_paths_open_the_viewer_and_subcommands_parse() {
        let cli = Cli::try_parse_from(["dicomscope-tui", "a.zip", "o.hl7"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.view.paths.len(), 2);

        let cli = Cli::try_parse_from([
            "dicomscope-tui",
            "render",
            "a.zip",
            "--window",
            "-600/1500",
            "-o",
            "x.png",
            "--backend",
            "metal",
        ])
        .unwrap();
        assert_eq!(cli.gpu.backend, Backend::Metal);
        let Some(Command::Render(r)) = cli.command else {
            panic!("render expected")
        };
        assert_eq!(r.window, Some((-600.0, 1500.0)));

        assert!(Cli::try_parse_from(["dicomscope-tui", "render", "a.zip"]).is_err());
    }

    #[test]
    fn the_order_is_the_last_hl7_path_or_the_flag() {
        let s = StudyAndOrder {
            paths: vec!["o.hl7".into(), "a.zip".into()],
            order: None,
        };
        assert_eq!(s.split().unwrap(), (vec!["a.zip".into()], "o.hl7".into()));
        let s = StudyAndOrder {
            paths: vec!["a.zip".into(), "msg".into()],
            order: None,
        };
        assert_eq!(s.split().unwrap().1, PathBuf::from("msg"));
        let s = StudyAndOrder {
            paths: vec!["o.hl7".into()],
            order: None,
        };
        assert!(s.split().is_err());
    }
}
