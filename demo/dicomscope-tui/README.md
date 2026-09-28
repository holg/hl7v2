# dicomscope-tui

dicomscope in the terminal. It is a [ratatui] viewer for DICOM studies and
the HL7 v2 orders they belong to. The images are rendered on a real GPU
backend through [wgpu] (Metal on macOS; Vulkan, DX12 or OpenGL elsewhere)
and shown inside the terminal with the kitty, iTerm2 or sixel graphics
protocols, or with Unicode half blocks on any other terminal.

With `--window` the same viewer opens in its own window instead: the
panels and keys stay the same, [ratatui-wgpu] draws the text on the GPU,
and the slice is composited into it at the window's full resolution. No
terminal graphics support is needed.

It is also a scriptable command-line tool. Each view in the viewer has a
subcommand that prints text or JSON: series listing, tag dump, GPU render
to PNG, order parsing, study/order linkage with a meaningful exit status,
FHIR R4 resources, the Modality Worklist item and embedded reports.

The browser app (`dicomscope`), the desktop and iPad app
(`dicomscope-desktop`) and this crate all run the same
`dicomscope-core` code: DICOM loading, the WGSL renderer, the linkage rules,
FHIR and worklist output. What you see in the terminal is what the other
apps show.

[ratatui]: https://ratatui.rs
[wgpu]: https://wgpu.rs
[ratatui-wgpu]: https://docs.rs/ratatui-wgpu

## Contents

- [Quick start](#quick-start)
- [How images reach the terminal](#how-images-reach-the-terminal)
- [Window mode](#window-mode)
- [The viewer](#the-viewer)
- [Keys](#keys)
- [Commands](#commands)
- [Global options](#global-options)
- [Exit status](#exit-status)
- [Environment](#environment)
- [As a library: image plugins](#as-a-library-image-plugins)

## Quick start

```sh
cargo install --path demo/dicomscope-tui     # or: cargo run -p dicomscope-tui --release --

dicomscope-tui samples/anonymized-demo.zip samples/anonymized-demo.hl7
```

Paths can be DICOM files, folders or zip archives, which are read together
as one study, and `.hl7`/`.txt` order messages, in any order. Press `?` in
the viewer for the keys and `q` to quit.

## How images reach the terminal

```text
 DICOM slice ──decode──► R32Float / RGBA8 texture
                              │  WGSL shader: VOI LUT window, zoom, pan,
                              │  rotate, flip, bilinear, local contrast
                              ▼
                 offscreen Rgba8UnormSrgb target (Metal / Vulkan / DX12 / GL)
                              │  copy_texture_to_buffer + map
                              ▼
                   RGBA image sized to the panel's pixels
                              │  kitty · iTerm2 · sixel · half blocks
                              ▼
                          your terminal
```

The image is rendered at exactly the pixel size of the panel it goes into:
cells × the terminal's cell size, which is asked from the terminal at
start. The terminal does no scaling. Windowing, zooming and every other
change re-render on the GPU, which usually takes a few milliseconds; the
status bar shows the time.

| Terminal | Protocol picked by `--protocol auto` |
|---|---|
| kitty, Ghostty, WezTerm | kitty graphics |
| iTerm2, WezTerm | iTerm2 inline images |
| foot, mlterm, xterm (`-ti vt340`), Windows Terminal ≥ 1.22 | sixel |
| everything else, including Terminal.app and tmux without passthrough | half blocks |

Press `p` in the viewer to try the next protocol. `--protocol` forces one.

## Window mode

```sh
dicomscope-tui --window samples/anonymized-demo.zip samples/anonymized-demo.hl7
```

In a terminal without graphics support (Terminal.app, Alacritty, most
SSH clients) the terminal can only show the half-block approximation.
`--window` (`-w`) sidesteps the terminal entirely:

```text
 ratatui UI ──► ratatui-wgpu: glyphs → text texture ──┐
                                                       │  Compositor (post-processor)
 DICOM slice ──► core renderer, same wgpu device ──────┤  1. blit the text to the surface
                                                       │  2. draw the slice into the
                                                       ▼     image panel's rectangle
                                              window surface (Metal / Vulkan / DX12 / GL)
```

Both halves share one device and one frame, and nothing is read back. The
image panel is drawn straight onto the window's surface at native
resolution, Retina included. The keys are the same except `p`, which has
no protocols to switch; `Cmd-Q` and `Cmd-W` also close the window. The
text uses the bundled Cascadia Mono (SIL Open Font License); `--font FILE`
takes any other TrueType or OpenType font, for example
`/System/Library/Fonts/Menlo.ttc`.

Window mode runs wherever winit and wgpu do: macOS (Metal), Windows (DX12,
Vulkan) and Linux on X11 or Wayland (Vulkan, GL). It needs a local desktop
session; over SSH, use the terminal mode.

## The viewer

```text
┌ PATIENT NAME ────────────┐ F1 Image  F2 Tags  F3 Order  F4 Linkage  F5 Worklist  F6 FHIR  F7 Documents
│▶ CT  Axial 2mm (140)     │┌ 1.2.3.dcm  ·  61/140  ·  512x512 ─────────────────────────────────────┐
│  CT  Coronal (98)        ││                                                                       │
│  SR  #900 (1)            ││                    GPU-rendered slice, inline                         │
├ Order ───────────────────┤│                                                                       │
│ ORM^O01 from RIS (…)     ││                                                                       │
│ Patient   12345          ││                                                                       │
│ Accession A-778          ││                                                                       │
│ ● linked via Study UID   │└───────────────────────────────────────────────────────────────────────┘
└──────────────────────────┘ series 1/3  slice 61/140 │ W 400 C 40 │ zoom 1.00 │ Metal · Apple M3 → kitty 4 ms
```

| Panel | Shows |
|---|---|
| Image | the slice, rendered by the GPU |
| Tags | the slice's DICOM header, sequences indented; `/` filters |
| Order | message type, the four linkage fields and their HL7 paths, warnings, and the raw message with those fields highlighted in colour |
| Linkage | the study against the order: link path, each identifier pair (`=` match, `≠` differs, `·` missing), key conflicts, and the order → worklist → image chain |
| Worklist | the Modality Worklist item the order becomes |
| FHIR | Bundle, Patient, ServiceRequest, ImagingStudy (`←` `→` to switch) |
| Documents | Structured Reports as text; encapsulated PDFs by size, with the command that saves them |

## Keys

| Key | Action |
|---|---|
| `Tab` / `Shift-Tab`, `F1`-`F7` | next / previous panel, or jump to one |
| `[` `]` | previous / next series (any panel) |
| `↑` `↓`, `k` `j`, mouse wheel | Image: previous / next slice; text panels: scroll |
| `←` `→`, `h` `l` | Image: previous / next series; FHIR: resource |
| `PgUp` `PgDn`, `Home` `End` | 10 slices / first, last (Image); page (text) |
| `+` `-` | zoom in / out about the centre |
| `0` `=` | fit to the panel / one image pixel per screen pixel |
| `H` `J` `K` `L` | pan left, down, up, right |
| `r` `R` | rotate 90° clockwise / counter-clockwise |
| `f` `v` | flip left-right / top-bottom |
| `w` `W` | narrower / wider window (more / less contrast) |
| `c` `C` | lower / raise window centre (brighter / darker) |
| `a` | window from the file again |
| `i` `e` `s` | invert, local contrast enhancement, sharp pixels |
| `Space` | cine play / pause (multi-frame and series) |
| `/` | Tags: filter; `Enter` keeps it, `Esc` clears it |
| `Enter` `Esc` | Documents: open / back |
| `p` | next graphics protocol (kitty, iTerm2, sixel, half blocks) |
| `?` `Esc` | this help; either closes it (so does any key or a click) |
| `q`, `Ctrl-C` | quit |

### Mouse

| Mouse | Action |
|---|---|
| click | a tab, a series, a document; anywhere closes the help |
| double click | Image: fit to the panel; Documents: open |
| wheel | Image: slices; series list: series; text panels: scroll |
| Option + wheel | Image: zoom at the pointer. Also Ctrl + wheel off the Mac (on a Mac, Ctrl + scroll is the system screen zoom), and in `--window` mode Cmd + wheel and trackpad pinch |
| left drag | Image: pan |
| right drag | Image: window width (left, right) and centre (up, down) |

The same in the terminal (any terminal with mouse reporting, which the
viewer turns on) and in `--window` mode. In the terminal, hold `Shift` (or
`Option` in iTerm2 and Terminal.app) to select text instead.

## Commands

```text
dicomscope-tui [OPTIONS] [PATH]...           the viewer
dicomscope-tui <COMMAND> [OPTIONS] <ARGS>    one job, then exit
```

| Command | Does |
|---|---|
| [`view`](#view) | the viewer (the default without a command) |
| [`info`](#info) | study, series, documents, skipped files |
| [`tags`](#tags) | the DICOM header of one slice |
| [`render`](#render) | one slice on the GPU to PNG or into the terminal |
| [`order`](#order) | parse an HL7 v2 order |
| [`link`](#link) | check a study against an order (exit 3 unless they link cleanly) |
| [`fhir`](#fhir) | FHIR R4 Bundle or single resources |
| [`worklist`](#worklist) | the Modality Worklist item, as text, JSON or a DICOM file |
| [`docs`](#docs) | list, print or extract SRs and PDFs |
| [`gpu`](#gpu) | GPU adapters per backend |

Commands that print data take `--json` for machine-readable output. Series
and slices are counted from 1, in the order `info` lists them. The complete
flag reference is in the [`cli`] module, and `dicomscope-tui <COMMAND>
--help` prints the same text.

### view

```text
dicomscope-tui [view] [PATH]... [--order FILE] [--series N] [--slice N]
               [--protocol auto|kitty|iterm2|sixel|halfblocks] [--backend B]
               [-w|--window [--font FILE]]
```

```sh
dicomscope-tui samples/study-demo.zip samples/order.hl7
dicomscope-tui ./CT-folder --order incoming.msg --series 2 --slice 40
dicomscope-tui scan.dcm --protocol halfblocks        # over SSH, in tmux, anywhere
dicomscope-tui -w study.zip order.hl7                # own window, full resolution
```

### info

```text
dicomscope-tui info <STUDY>... [--json]
```

Patient, identifiers, date, description and transfer syntax. Then each
series with modality, description, slice count and matrix, the documents,
and up to ten skipped files with the reason they were skipped.

```sh
dicomscope-tui info samples/study-demo.zip
dicomscope-tui info ./export --json | jq '.series[] | {index, description, slices}'
```

### tags

```text
dicomscope-tui tags <STUDY>... [--series N] [--slice N] [-g|--grep TEXT] [--json]
```

One row per element: `(gggg,eeee) VR Keyword value`, with sequence items
indented. `--grep` keeps rows whose tag, keyword, VR or value contains the
text, ignoring case.

```sh
dicomscope-tui tags scan.dcm --grep patient
dicomscope-tui tags study.zip --series 2 --slice 10 --json > tags.json
```

### render

```text
dicomscope-tui render <STUDY>... (-o FILE.png | --inline) [--series N] [--slice N]
               [--size WxH] [--window C/W] [--zoom F] [--rotate 0-3]
               [--flip-h] [--flip-v] [--invert] [--enhance] [--sharp]
               [--protocol P] [--backend B]
```

Renders one slice with the viewer's GPU pipeline. Without `--size` the
output has the image's own size (rotated if needed); with it, the image is
fitted inside and centred on black. `--window` takes centre/width in
stored units after rescale (Hounsfield for CT); without it, the file's
first window is used, else the full value range. `--inline` prints the
result into the terminal's scrollback.

```sh
dicomscope-tui render ct.zip --series 2 --slice 60 --window 40/400 -o soft-tissue.png
dicomscope-tui render ct.zip --window -600/1500 --size 1024x1024 -o lung.png
dicomscope-tui render opg.dcm --enhance --inline
dicomscope-tui render mr.zip --rotate 1 --flip-h --zoom 2 -o detail.png --backend vulkan
```

### order

```text
dicomscope-tui order <ORDER> [--raw] [--json]
```

Message type, sender, version and control ID. Then the identifiers used
for linkage, each with the HL7 path it came from, whether a worklist item
can be built, and any warnings (disagreeing study UIDs, Latin-1 input and
so on). `--raw` adds the message, one segment per line.

```sh
dicomscope-tui order samples/order.hl7
dicomscope-tui order samples/order-oru.hl7 --json
```

### link

```text
dicomscope-tui link <STUDY>... <ORDER> [--json]
dicomscope-tui link <STUDY>... --order FILE [--json]
```

The order is the `--order` file, else the last `.hl7`/`.txt` path, else
the last path. Prints the verdict and link path (Study Instance UID, else
Accession Number), each identifier pair (`=`, `≠`, `·` for absent), key
conflicts, and the order → worklist → image chain with its findings.

Exits with **3** when the study and order do not link, a key conflicts, or
they link but the patient IDs differ (a study filed under the wrong
patient). That makes it usable as a pipeline gate:

```sh
if dicomscope-tui link incoming/ order.hl7 --json > link.json; then
  echo "archive"
else
  echo "hold for reconciliation"
fi
```

### fhir

```text
dicomscope-tui fhir <STUDY>... <ORDER> [-r bundle|patient|service-request|imaging-study] [-o FILE]
```

FHIR R4 JSON. The Bundle is a transaction with Patient (from PID),
ServiceRequest (from ORC/OBR/IPC) and ImagingStudy (from the DICOM study),
cross-referenced.

```sh
dicomscope-tui fhir study.zip order.hl7 > bundle.json
dicomscope-tui fhir study.zip order.hl7 -r imaging-study -o imagingstudy.json
```

### worklist

```text
dicomscope-tui worklist <ORDER> [-o FILE.dcm] [--json]
```

The DICOM Modality Worklist item built from the order. Without `-o` it
prints the item's elements; with `-o` it writes a Part 10 file that a
worklist SCP can serve.

```sh
dicomscope-tui worklist samples/order-omi.hl7
dicomscope-tui worklist samples/order-omi.hl7 -o mwl/item.dcm
```

### docs

```text
dicomscope-tui docs <STUDY>... [--show N [-o FILE]] [--json]
```

Lists Structured Reports, Key Object Selections and encapsulated PDFs.
`--show N` prints report N as text or writes it with `-o`; a PDF needs
`-o`.

```sh
dicomscope-tui docs study.zip
dicomscope-tui docs study.zip --show 2 -o letter.pdf
```

### gpu

```text
dicomscope-tui gpu [--backend B]
```

Every adapter wgpu finds on every backend, with device type and driver,
then the adapter `--backend` would pick.

## Global options

| Option | Values | Default |
|---|---|---|
| `--backend` | `auto`, `metal`, `vulkan`, `dx12`, `gl` | `auto`: Metal on macOS, Vulkan or DX12 elsewhere, GL last |
| `--protocol` (viewer, `render --inline`) | `auto`, `kitty`, `iterm2`, `sixel`, `halfblocks` | `auto`: asks the terminal |
| `-w`, `--window` (viewer) | | off: the viewer runs in the terminal |
| `--font` (with `--window`) | a `.ttf`, `.otf` or `.ttc` file | the bundled Cascadia Mono |
| `-h`, `--help` | short help; `--help` shows the long form | |
| `-V`, `--version` | | |

## Exit status

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | an error: unreadable input, no image, no GPU, bad series number … (the message goes to stderr) |
| 2 | invalid command line |
| 3 | `link`: the study and the order do not link, a key conflicts, or the patient IDs differ |

## Environment

| Variable | Effect |
|---|---|
| `WGPU_BACKEND` | with `--backend auto`, restricts wgpu to these backends (`metal`, `vulkan`, `dx12`, `gl`, comma-separated) |
| `TERM`, `TERM_PROGRAM`, `TMUX` | used by the protocol detection |

## As a library: image plugins

The viewer never renders anything itself. It asks an
[`ImagePlugin`](gpu::ImagePlugin) for an RGBA image of a given pixel size
and hands it to the terminal. [`WgpuPlugin`](gpu::WgpuPlugin) is the
implementation that ships. It wraps `dicomscope_core::render::Renderer`,
which draws into an offscreen texture and reads it back. The same
renderer draws into a window surface in the desktop app and a canvas in
the browser.

A different renderer (a CPU fallback, a remote render server, a
recording plugin for tests) implements the trait:

```rust
use dicomscope_core::session::Session;
use dicomscope_core::AppError;
use dicomscope_tui::gpu::{ImagePlugin, Look};

/// Grey test card: every pixel mid-grey.
struct TestCard;

impl ImagePlugin for TestCard {
    fn describe(&self) -> String {
        "test card".into()
    }
    fn render(
        &mut self,
        _session: &mut Session,
        (w, h): (u32, u32),
        _look: Look,
    ) -> Result<image::RgbaImage, AppError> {
        Ok(image::RgbaImage::from_pixel(w, h, image::Rgba([128, 128, 128, 255])))
    }
}

let mut plugin = TestCard;
let picture = plugin.render(&mut Session::new(), (4, 3), Look::default()).unwrap();
assert_eq!(picture.dimensions(), (4, 3));
```

## Not a medical device

This is a demonstration viewer. It is not validated for diagnosis and must
not be used for it.
