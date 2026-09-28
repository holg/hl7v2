//! Drawing the viewer: a sidebar with the series and the order, tabs on the
//! right, a status bar at the bottom. Pure functions of the [`App`].

use crate::app::{App, Mode, Tab, FHIR_RESOURCES};
use crate::commands::{filter_tags, is_linked, link_text};
use crate::term;
use dicomscope_core::session::DocumentContent;
use hl7kit::order::OrderField;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use ratatui::Frame;
use ratatui_image::Image;

/// Where everything goes, computed once per frame so the image can be
/// rendered for its area before drawing starts, and kept for mouse hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Areas {
    /// The series list.
    pub series: Rect,
    /// The order summary and link verdict.
    pub order: Rect,
    /// The tab bar.
    pub tabs: Rect,
    /// The active panel, border included.
    pub body: Rect,
    /// Inside the body's border: the image, in cells.
    pub image: Rect,
    /// The status bar.
    pub status: Rect,
}

/// Split the terminal into [`Areas`].
pub fn layout(full: Rect) -> Areas {
    let [main, status] = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(full);
    let side_w = if main.width >= 100 { 36 } else { 28 };
    let [side, right] =
        Layout::horizontal([Constraint::Length(side_w), Constraint::Min(10)]).areas(main);
    let [series, order] =
        Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(side);
    let [tabs, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(right);
    let image = Block::bordered().inner(body);
    Areas {
        series,
        order,
        tabs,
        body,
        image,
        status,
    }
}

const ACCENT: Color = Color::Cyan;

fn block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().dark_gray())
        .title(Span::styled(
            format!(" {title} "),
            Style::new().fg(ACCENT).bold(),
        ))
}

/// Draw one frame. The image must already be rendered for `a.image`.
pub fn draw(f: &mut Frame, app: &App, a: &Areas) {
    draw_series(f, app, a.series);
    draw_order_summary(f, app, a.order);
    draw_tabs(f, app, a.tabs);
    match app.tab {
        Tab::Image => draw_image(f, app, a),
        Tab::Tags => draw_tags(f, app, a.body),
        Tab::Order => draw_order(f, app, a.body),
        Tab::Link => draw_link(f, app, a.body),
        Tab::Worklist => draw_worklist(f, app, a.body),
        Tab::Fhir => draw_fhir(f, app, a.body),
        Tab::Docs => draw_docs(f, app, a.body),
    }
    draw_status(f, app, a.status);
    if app.help {
        draw_help(f, f.area());
    }
}

fn draw_series(f: &mut Frame, app: &App, area: Rect) {
    let s = &app.session;
    let title = s
        .study
        .as_ref()
        .and_then(|st| st.patient_name.clone())
        .map(|n| n.replace('^', " "))
        .unwrap_or_else(|| "Study".into());
    let Some(set) = &s.set else {
        let hint = Paragraph::new(vec![
            Line::from("No study open."),
            Line::from(""),
            Line::from("dicomscope-tui STUDY [ORDER]").dark_gray(),
        ])
        .wrap(Wrap { trim: true })
        .block(block(&title));
        f.render_widget(hint, area);
        return;
    };
    let items: Vec<ListItem> = set
        .series
        .iter()
        .map(|sr| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<3}", sr.modality),
                    Style::new().fg(Color::Yellow),
                ),
                Span::raw(if sr.description.is_empty() {
                    format!("#{}", sr.number.unwrap_or(0))
                } else {
                    sr.description.clone()
                }),
                Span::styled(format!(" ({})", sr.slices.len()), Style::new().dark_gray()),
            ]))
        })
        .collect();
    let mut state = ListState::default().with_selected(Some(s.current.0));
    let list = List::new(items)
        .block(block(&title))
        .highlight_style(Style::new().bg(Color::DarkGray).bold())
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_order_summary(f: &mut Frame, app: &App, area: Rect) {
    let s = &app.session;
    let mut lines = Vec::new();
    match &s.hl7 {
        None => {
            lines.push(Line::from("No order open.").dark_gray());
            lines.push(Line::from("Pass an .hl7 file or --order.").dark_gray());
        }
        Some(h) => {
            lines.push(Line::from(h.summary.clone()));
            lines.push(Line::from(""));
            for (label, v) in [
                ("Patient", &h.order.patient_id),
                ("Accession", &h.order.accession),
                ("Procedure", &h.order.procedure_id),
                ("Study UID", &h.order.study_uid),
            ] {
                lines.push(Line::from(vec![
                    Span::styled(format!("{label:<10}"), Style::new().dark_gray()),
                    Span::raw(v.clone().unwrap_or_else(|| "—".into())),
                ]));
            }
            lines.push(Line::from(""));
            lines.push(match &s.linkage {
                Some(l) if is_linked(l) && !l.linked_with_patient_mismatch() => {
                    Line::from(format!("● linked via {}", short_path(l)))
                        .green()
                        .bold()
                }
                Some(l) if is_linked(l) => {
                    Line::from("● linked, patient IDs differ").yellow().bold()
                }
                Some(_) => Line::from("● not linked to this study").red().bold(),
                None => Line::from("○ no study to link").dark_gray(),
            });
            if !h.warnings.is_empty() {
                lines.push(Line::from(format!("{} warning(s)", h.warnings.len())).yellow());
            }
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block("Order")),
        area,
    );
}

fn short_path(l: &dicomscope_core::link::Linkage) -> &'static str {
    match l.path {
        dicomscope_core::link::LinkPath::StudyUid => "Study UID",
        dicomscope_core::link::LinkPath::Accession => "Accession",
        dicomscope_core::link::LinkPath::None => "nothing",
    }
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| Line::from(tab_label(i, *t)))
        .collect();
    let selected = Tab::ALL.iter().position(|t| *t == app.tab);
    f.render_widget(
        Tabs::new(titles)
            .select(selected)
            .style(Style::new().dark_gray())
            .highlight_style(Style::new().fg(ACCENT).bold().underlined())
            .divider(" "),
        area,
    );
}

fn draw_image(f: &mut Frame, app: &App, a: &Areas) {
    let s = &app.session;
    let title = match (&s.set, s.frame.as_ref()) {
        (Some(set), Some(fr)) => {
            let (si, sl) = s.current;
            let n = set.series.get(si).map_or(0, |x| x.slices.len());
            format!(
                "{}  ·  {}/{}  ·  {}x{}",
                base_name(&s.file_name),
                sl + 1,
                n,
                fr.width,
                fr.height
            )
        }
        _ => "Image".into(),
    };
    f.render_widget(block(&title), a.body);
    let terminal_mode = matches!(app.mode, Mode::Terminal(_));
    let message = if let (true, Err(e)) = (terminal_mode, &app.plugin) {
        Some(format!(
            "No GPU adapter for the image: {e}\n\nTry --backend gl, or `dicomscope-tui gpu`."
        ))
    } else if let Some(e) = &app.render_error {
        Some(format!("Rendering failed: {e}"))
    } else if s.set.is_none() {
        Some("Open a study: dicomscope-tui STUDY [ORDER]\n\nPress ? for keys.".into())
    } else if s.frame.is_none() {
        Some(
            s.error
                .clone()
                .unwrap_or_else(|| "This slice has no displayable image.".into()),
        )
    } else {
        None
    };
    if let Some(m) = message {
        f.render_widget(
            Paragraph::new(m)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            centered(a.image, 90, 5),
        );
        return;
    }
    // Terminal graphics sit above the text layer: the help box would be
    // hidden under the image, so the image waits until help closes.
    if let (Some((area, g)), false) = (&app.graphic, app.help) {
        f.render_widget(Image::new(g), *area);
    }
}

fn draw_tags(f: &mut Frame, app: &App, area: Rect) {
    let filter = (!app.tag_filter.is_empty() || app.filtering).then_some(app.tag_filter.as_str());
    let rows = filter_tags(&app.session.tags, filter);
    let title = match (app.filtering, filter) {
        (true, _) => format!("Tags  /{}▏", app.tag_filter),
        (false, Some(q)) => format!("Tags  /{q}  ({} rows)", rows.len()),
        (false, None) => format!("Tags  ({} rows, / to filter)", rows.len()),
    };
    let lines: Vec<Line> = rows
        .iter()
        .map(|r| {
            Line::from(vec![
                Span::raw("  ".repeat(r.depth)),
                Span::styled(r.tag.clone(), Style::new().fg(Color::Yellow)),
                Span::raw(" "),
                Span::styled(format!("{:<2}", r.vr), Style::new().dark_gray()),
                Span::raw(" "),
                Span::styled(format!("{:<34}", r.keyword), Style::new().fg(ACCENT)),
                Span::raw(" "),
                Span::raw(r.value.clone()),
            ])
        })
        .collect();
    scrolled(f, app, area, &title, lines, false);
}

fn field_color(field: OrderField) -> Color {
    match field {
        OrderField::PatientId => Color::Cyan,
        OrderField::Accession => Color::Yellow,
        OrderField::ProcedureId => Color::Magenta,
        OrderField::StudyUid => Color::Green,
    }
}

fn draw_order(f: &mut Frame, app: &App, area: Rect) {
    let Some(h) = &app.session.hl7 else {
        f.render_widget(
            Paragraph::new("No order open. Pass an .hl7 file, or --order FILE.")
                .block(block("Order")),
            area,
        );
        return;
    };
    let mut lines = vec![Line::from(h.summary.clone()).bold(), Line::from("")];
    for field in OrderField::ALL {
        let value = match field {
            OrderField::PatientId => &h.order.patient_id,
            OrderField::Accession => &h.order.accession,
            OrderField::ProcedureId => &h.order.procedure_id,
            OrderField::StudyUid => &h.order.study_uid,
        };
        lines.push(Line::from(vec![
            Span::styled("■ ", Style::new().fg(field_color(field))),
            Span::styled(format!("{:<24}", field.label()), Style::new().bold()),
            Span::styled(
                format!("{:<18}", field.paths().join(" | ")),
                Style::new().dark_gray(),
            ),
            Span::raw(value.clone().unwrap_or_else(|| "—".into())),
        ]));
    }
    for w in &h.warnings {
        lines.push(Line::from(format!("! {w}")).yellow());
    }
    lines.push(Line::from(""));
    lines.extend(highlighted_message(&h.raw, &h.spans));
    scrolled(
        f,
        app,
        area,
        &format!("Order  {}", base_name(&h.name)),
        lines,
        false,
    );
}

/// The raw message, one segment per line, with the order fields coloured.
fn highlighted_message(raw: &str, spans: &[(hl7kit::Span, OrderField)]) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, ch) in raw.char_indices().chain(std::iter::once((raw.len(), '\r'))) {
        if ch != '\r' && ch != '\n' {
            continue;
        }
        if i > start {
            let mut parts = Vec::new();
            let mut pos = start;
            let mut marks: Vec<_> = spans
                .iter()
                .filter(|(s, _)| s.start >= start && s.end <= i && s.start < s.end)
                .collect();
            marks.sort_by_key(|(s, _)| s.start);
            for (s, field) in marks {
                if s.start < pos || !raw.is_char_boundary(s.start) || !raw.is_char_boundary(s.end) {
                    continue;
                }
                parts.push(Span::raw(raw[pos..s.start].to_string()));
                parts.push(Span::styled(
                    raw[s.start..s.end].to_string(),
                    Style::new()
                        .fg(Color::Black)
                        .bg(field_color(*field))
                        .add_modifier(Modifier::BOLD),
                ));
                pos = s.end;
            }
            parts.push(Span::raw(raw[pos..i].to_string()));
            out.push(Line::from(parts));
        }
        start = i + ch.len_utf8();
    }
    out
}

fn draw_link(f: &mut Frame, app: &App, area: Rect) {
    let s = &app.session;
    let text = match &s.linkage {
        Some(l) => link_text(l, s.chain.as_ref()),
        None if s.hl7.is_none() => "Open an order to check it against the study.".into(),
        None => "Open a study to check the order against.".into(),
    };
    let lines: Vec<Line> = text
        .lines()
        .map(|l| {
            let line = Line::from(l.to_string());
            if l.starts_with("LINKED") {
                line.green().bold()
            } else if l.starts_with("NOT LINKED") {
                line.red().bold()
            } else if l.trim_start().starts_with('!') {
                line.yellow()
            } else if l.contains('≠') {
                line.red()
            } else {
                line
            }
        })
        .collect();
    scrolled(f, app, area, "Linkage", lines, false);
}

fn draw_worklist(f: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line> = match &app.session.worklist {
        None => vec![Line::from(
            "Open an order to build its Modality Worklist item.",
        )],
        Some(Err(e)) => vec![Line::from(format!("No worklist item: {e}")).red()],
        Some(Ok(w)) => w
            .rows
            .iter()
            .map(|r| {
                Line::from(vec![
                    Span::raw("  ".repeat(r.depth)),
                    Span::styled(r.tag.clone(), Style::new().fg(Color::Yellow)),
                    Span::raw(" "),
                    Span::styled(format!("{:<2}", r.vr), Style::new().dark_gray()),
                    Span::raw(" "),
                    Span::styled(format!("{:<34}", r.keyword), Style::new().fg(ACCENT)),
                    Span::raw(" "),
                    Span::raw(r.value.clone()),
                ])
            })
            .collect(),
    };
    scrolled(f, app, area, "Modality Worklist item", lines, false);
}

fn draw_fhir(f: &mut Frame, app: &App, area: Rect) {
    let name = FHIR_RESOURCES[app.fhir_resource];
    let title = format!("FHIR R4  ◀ {name} ▶");
    let Some(fh) = &app.session.fhir else {
        let why = if app.session.hl7.is_none() {
            "Open an order: FHIR output needs the study and its order."
        } else {
            "Open a study: FHIR output needs the study and its order."
        };
        f.render_widget(Paragraph::new(why).block(block(&title)), area);
        return;
    };
    let json = match app.fhir_resource {
        0 => &fh.bundle,
        1 => &fh.patient,
        2 => &fh.service_request,
        _ => &fh.imaging_study,
    };
    let lines: Vec<Line> = json.lines().map(json_line).collect();
    scrolled(f, app, area, &title, lines, false);
}

/// Colour a pretty-printed JSON line: keys and string values.
fn json_line(l: &str) -> Line<'static> {
    let indent = l.len() - l.trim_start().len();
    let body = l.trim_start();
    if let Some(rest) = body.strip_prefix('"') {
        if let Some(end) = rest.find("\": ") {
            let key = &rest[..end];
            let value = &rest[end + 3..];
            let vstyle = if value.starts_with('"') {
                Style::new().fg(Color::Green)
            } else {
                Style::new().fg(Color::Magenta)
            };
            return Line::from(vec![
                Span::raw(" ".repeat(indent)),
                Span::styled(format!("\"{key}\""), Style::new().fg(ACCENT)),
                Span::raw(": "),
                Span::styled(value.to_string(), vstyle),
            ]);
        }
        return Line::from(Span::styled(l.to_string(), Style::new().fg(Color::Green)));
    }
    Line::from(l.to_string())
}

fn draw_docs(f: &mut Frame, app: &App, area: Rect) {
    let s = &app.session;
    if let Some((_, doc)) = &s.document {
        let (title, lines): (String, Vec<Line>) = match doc {
            DocumentContent::Report { title, text } => (
                format!("{title}  (Esc: back)"),
                text.lines().map(|l| Line::from(l.to_string())).collect(),
            ),
            DocumentContent::Pdf { title, bytes, mime } => (
                format!("{title}  (Esc: back)"),
                vec![
                    Line::from(format!("{mime}, {} bytes.", bytes.len())),
                    Line::from(""),
                    Line::from("A terminal cannot show a PDF. Save it with:"),
                    Line::from(format!(
                        "  dicomscope-tui docs STUDY --show {} -o document.pdf",
                        app.doc_selected + 1
                    ))
                    .fg(ACCENT),
                ],
            ),
            DocumentContent::Failed(e) => (
                "Document  (Esc: back)".into(),
                vec![Line::from(e.clone()).red()],
            ),
        };
        scrolled(f, app, area, &title, lines, true);
        return;
    }
    let docs = s
        .set
        .as_ref()
        .map(|x| x.documents.as_slice())
        .unwrap_or(&[]);
    if docs.is_empty() {
        f.render_widget(
            Paragraph::new("No Structured Reports or PDFs in this study.")
                .block(block("Documents")),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = docs.iter().map(|d| ListItem::new(d.label())).collect();
    let mut state = ListState::default().with_selected(Some(app.doc_selected));
    f.render_stateful_widget(
        List::new(items)
            .block(block("Documents  (Enter: open)"))
            .highlight_style(Style::new().bg(Color::DarkGray).bold())
            .highlight_symbol("▶ "),
        area,
        &mut state,
    );
}

/// A bordered, scrollable text panel. The scroll offset is clamped here,
/// where the height is known.
fn scrolled(f: &mut Frame, app: &App, area: Rect, title: &str, lines: Vec<Line>, wrap: bool) {
    let height = area.height.saturating_sub(2) as usize;
    let max = lines.len().saturating_sub(height);
    let offset =
        (app.scroll[Tab::ALL.iter().position(|t| *t == app.tab).unwrap_or(0)] as usize).min(max);
    let title = if max > 0 {
        format!(
            "{title}  [{}-{} of {}]",
            offset + 1,
            (offset + height).min(lines.len()),
            lines.len()
        )
    } else {
        title.to_string()
    };
    let mut p = Paragraph::new(Text::from(lines))
        .block(block(&title))
        .scroll((offset as u16, 0));
    if wrap {
        p = p.wrap(Wrap { trim: false });
    }
    f.render_widget(p, area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let s = &app.session;
    if let Some(e) = &s.error {
        if s.frame.is_some() || s.set.is_none() {
            f.render_widget(Paragraph::new(format!(" {e}")).red(), area);
            return;
        }
    }
    let mut parts: Vec<Span> = Vec::new();
    let sep = || Span::styled("  │  ", Style::new().dark_gray());
    if let Some(note) = &app.note {
        parts.push(Span::styled(
            format!(" {note}"),
            Style::new().fg(Color::Yellow),
        ));
    } else {
        if let Some(set) = &s.set {
            let (si, sl) = s.current;
            let n = set.series.get(si).map_or(0, |x| x.slices.len());
            parts.push(Span::raw(format!(
                " series {}/{}  slice {}/{}",
                si + 1,
                set.series.len(),
                sl + 1,
                n
            )));
            if s.frame.as_ref().is_some_and(|f| !f.color) {
                parts.push(sep());
                parts.push(Span::raw(format!(
                    "W {:.0} C {:.0}",
                    s.window.1, s.window.0
                )));
            }
            parts.push(sep());
            parts.push(Span::raw(format!("zoom {:.2}", s.view.scale)));
            let mut flags = Vec::new();
            if s.playing {
                flags.push("▶ cine");
            }
            if app.look.invert {
                flags.push("inverted");
            }
            if s.enhance {
                flags.push("enhanced");
            }
            if !s.view.smooth {
                flags.push("sharp");
            }
            if !flags.is_empty() {
                parts.push(sep());
                parts.push(Span::raw(flags.join(" ")));
            }
            parts.push(sep());
        } else {
            parts.push(Span::raw(" "));
        }
        match (&app.mode, &app.plugin) {
            (Mode::Window(shared), _) => parts.push(Span::styled(
                format!("{} → window", shared.borrow().adapter),
                Style::new().fg(ACCENT),
            )),
            (Mode::Terminal(picker), Ok(p)) => parts.push(Span::styled(
                format!(
                    "{} → {} {:.0} ms",
                    p.describe(),
                    term::protocol_name(picker),
                    app.render_ms
                ),
                Style::new().fg(ACCENT),
            )),
            (Mode::Terminal(_), Err(_)) => parts.push(Span::styled("no GPU", Style::new().red())),
        }
    }
    parts.push(sep());
    parts.push(Span::styled("? keys  q quit", Style::new().dark_gray()));
    f.render_widget(Paragraph::new(Line::from(parts)), area);
}

/// Every key binding, shown by `?` and printed in the crate docs.
pub const KEYS: &[(&str, &str)] = &[
    (
        "Tab / Shift-Tab, F1-F7",
        "next / previous panel, or jump to one",
    ),
    ("[  ]", "previous / next series (any panel)"),
    (
        "↑ ↓  k j, wheel",
        "Image: previous / next slice; text panels: scroll",
    ),
    ("← →  h l", "Image: previous / next series; FHIR: resource"),
    (
        "PgUp PgDn, Home End",
        "10 slices / first, last (Image); page (text)",
    ),
    ("+  -", "zoom in / out about the centre"),
    (
        "0  =",
        "fit to the panel / one image pixel per screen pixel",
    ),
    ("H J K L", "pan left, down, up, right"),
    ("r  R", "rotate 90° clockwise / counter-clockwise"),
    ("f  v", "flip left-right / top-bottom"),
    ("w  W", "narrower / wider window (more / less contrast)"),
    ("c  C", "lower / raise window centre (brighter / darker)"),
    ("a", "window from the file again"),
    (
        "i  e  s",
        "invert, local contrast enhancement, sharp pixels",
    ),
    ("Space", "cine play / pause (multi-frame and series)"),
    ("/", "Tags: filter; Enter keeps it, Esc clears it"),
    ("Enter  Esc", "Documents: open / back"),
    (
        "p",
        "next graphics protocol (kitty, iTerm2, sixel, half blocks)",
    ),
    (
        "?  Esc",
        "this help; either closes it (so does any key or a click)",
    ),
    ("q, Ctrl-C", "quit"),
];

/// What the mouse does, shown by `?` next to [`KEYS`].
pub const MOUSE: &[(&str, &str)] = &[
    (
        "click",
        "a tab, a series, a document; anywhere closes this help",
    ),
    ("double click", "Image: fit to the panel; Documents: open"),
    (
        "wheel",
        "Image: slices; series list: series; text panels: scroll",
    ),
    (
        "Option + wheel",
        "Image: zoom at the pointer (also Ctrl, Cmd, pinch)",
    ),
    ("left drag", "Image: pan"),
    ("right drag", "Image: window width (← →) and centre (↑ ↓)"),
];

fn draw_help(f: &mut Frame, full: Rect) {
    let row = |(k, what): &(&str, &str)| {
        Line::from(vec![
            Span::styled(format!(" {k:<24}"), Style::new().fg(ACCENT).bold()),
            Span::raw(what.to_string()),
        ])
    };
    let mut lines: Vec<Line> = KEYS.iter().map(row).collect();
    lines.push(Line::from(""));
    lines.push(Line::from(" Mouse").bold());
    lines.extend(MOUSE.iter().map(row));
    let area = centered(full, 80, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(block("Keys and mouse  (? or Esc closes)")),
        area,
    );
}

/// The last path component: a zip member or file name, not the whole path.
fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A `w` x `h` rectangle centred in `r`, clipped to it.
fn centered(r: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(r.width);
    let h = h.min(r.height);
    Rect::new(r.x + (r.width - w) / 2, r.y + (r.height - h) / 2, w, h)
}

fn tab_label(i: usize, t: Tab) -> String {
    format!("F{} {}", i + 1, t.title())
}

/// The tab under column `x` of the tab bar, laid out as [`Tabs`] draws
/// them: one cell of padding each side of a label, a one-cell divider.
pub fn tab_at(area: Rect, x: u16) -> Option<Tab> {
    let mut left = area.x;
    for (i, t) in Tab::ALL.iter().enumerate() {
        let w = tab_label(i, *t).chars().count() as u16 + 2;
        if (left..left + w).contains(&x) {
            return Some(*t);
        }
        left += w + 1;
    }
    None
}

/// The item under screen row `y` of a bordered list of `len` items drawn
/// with `selected` highlighted, scrolled as [`List`] scrolls it: just far
/// enough that the selection is on screen.
pub fn list_row_at(area: Rect, y: u16, len: usize, selected: usize) -> Option<usize> {
    let inner = Block::bordered().inner(area);
    if !(inner.y..inner.y + inner.height).contains(&y) {
        return None;
    }
    let offset = (selected + 1).saturating_sub(usize::from(inner.height));
    let i = offset + usize::from(y - inner.y);
    (i < len).then_some(i)
}

/// `true` when the cell (`x`, `y`) is inside `r`.
pub fn inside(r: Rect, x: u16, y: u16) -> bool {
    (r.x..r.x + r.width).contains(&x) && (r.y..r.y + r.height).contains(&y)
}
