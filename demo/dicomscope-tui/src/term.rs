//! Terminal graphics: choosing a protocol and putting an RGBA image on
//! screen, either inside the viewer or inline in the scrollback.

use crate::cli::Protocol;
use image::{DynamicImage, RgbaImage};
use ratatui::layout::Size;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::{FontSize, Image, Resize};

/// A picker for `protocol`. The terminal is asked for its cell size and
/// graphics support once; a terminal that does not answer (a pipe, an old
/// emulator) gets half blocks.
pub fn picker(protocol: Protocol) -> Picker {
    let mut p = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let forced = match protocol {
        Protocol::Auto => None,
        Protocol::Kitty => Some(ProtocolType::Kitty),
        Protocol::Iterm2 => Some(ProtocolType::Iterm2),
        Protocol::Sixel => Some(ProtocolType::Sixel),
        Protocol::Halfblocks => Some(ProtocolType::Halfblocks),
    };
    if let Some(t) = forced {
        p.set_protocol_type(t);
    }
    p
}

/// A short protocol name for the status bar.
pub fn protocol_name(p: &Picker) -> &'static str {
    match p.protocol_type() {
        ProtocolType::Halfblocks => "half blocks",
        ProtocolType::Sixel => "sixel",
        ProtocolType::Kitty => "kitty",
        ProtocolType::Iterm2 => "iTerm2",
    }
}

/// Pixels covered by `cols` x `rows` cells at the terminal's font size.
/// Half blocks get the same resolution and scale it down themselves.
pub fn cell_pixels(p: &Picker, cols: u16, rows: u16) -> (u32, u32) {
    let FontSize { width, height } = p.font_size();
    (
        u32::from(cols) * u32::from(width.max(1)),
        u32::from(rows) * u32::from(height.max(1)),
    )
}

/// Print an image into the scrollback, as wide as the terminal at most,
/// and move the cursor below it.
pub fn print_inline(picture: RgbaImage, protocol: Protocol) -> std::io::Result<()> {
    let picker = picker(protocol);
    let (tw, _) = crossterm::terminal::size().unwrap_or((80, 24));
    let (cw, ch) = cell_pixels(&picker, 1, 1);
    let cols = (picture.width().div_ceil(cw.max(1)) as u16).clamp(1, tw.max(1));
    let rows =
        ((picture.height() as f32 / picture.width().max(1) as f32 * f32::from(cols) * cw as f32
            / ch.max(1) as f32)
            .ceil() as u16)
            .max(1);
    let proto = picker
        .new_protocol(
            DynamicImage::ImageRgba8(picture),
            Size::new(cols, rows),
            Resize::Fit(None),
        )
        .map_err(std::io::Error::other)?;
    let mut terminal = ratatui::Terminal::with_options(
        ratatui::backend::CrosstermBackend::new(std::io::stdout()),
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Inline(rows),
        },
    )?;
    terminal.draw(|f| f.render_widget(Image::new(&proto), f.area()))?;
    println!();
    Ok(())
}
