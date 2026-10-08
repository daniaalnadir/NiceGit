use egui::{Color32, RichText, TextStyle, Ui};
use nicegit_core::diff::{side_by_side, DiffLine, DiffLineKind, SplitRow};

pub struct DiffContent {
    pub title: String,
    pub lines: Vec<DiffLine>,
    pub split: Vec<SplitRow>,
}

impl DiffContent {
    pub fn new(title: String, lines: Vec<DiffLine>) -> Self {
        let split = side_by_side(&lines);
        Self { title, lines, split }
    }

    pub fn is_empty(&self) -> bool {
        !self.lines.iter().any(|line| matches!(line.kind, DiffLineKind::Addition | DiffLineKind::Deletion | DiffLineKind::Context))
    }
}

fn colors(ui: &Ui, kind: DiffLineKind) -> (Color32, Color32) {
    let dark = ui.visuals().dark_mode;
    let text = ui.visuals().text_color();
    match kind {
        DiffLineKind::Addition => (if dark { Color32::from_rgb(0x1d, 0x3d, 0x2a) } else { Color32::from_rgb(0xdc, 0xf5, 0xe3) }, text),
        DiffLineKind::Deletion => (if dark { Color32::from_rgb(0x4a, 0x22, 0x26) } else { Color32::from_rgb(0xfb, 0xe0, 0xe2) }, text),
        DiffLineKind::Hunk => (ui.visuals().faint_bg_color, ui.visuals().weak_text_color()),
        DiffLineKind::Metadata => (Color32::TRANSPARENT, ui.visuals().weak_text_color()),
        DiffLineKind::Context => (Color32::TRANSPARENT, text),
    }
}

fn number(value: Option<usize>) -> String {
    value.map(|n| format!("{n:>5}")).unwrap_or_else(|| "     ".to_string())
}

/// Paints one line across the full width with its change colour.
fn line(ui: &mut Ui, diff_line: Option<&DiffLine>, numbers: bool, width: f32) {
    let height = ui.text_style_height(&TextStyle::Monospace);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let Some(diff_line) = diff_line else {
        ui.painter().rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        return;
    };
    let (background, text_color) = colors(ui, diff_line.kind);
    ui.painter().rect_filled(rect, 0.0, background);
    let mut text = String::new();
    if numbers {
        text.push_str(&number(diff_line.old_number));
        text.push(' ');
        text.push_str(&number(diff_line.new_number));
        text.push_str("  ");
    }
    // Tabs render as a single glyph box in egui; show them as spaces.
    text.push_str(&diff_line.text.replace('\t', "    "));
    let font = TextStyle::Monospace.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text, font, text_color);
    ui.painter().with_clip_rect(rect).galley(rect.left_top() + egui::vec2(4.0, 0.0), galley, text_color);
}

pub fn show(ui: &mut Ui, content: &DiffContent, split: bool) {
    if content.lines.is_empty() || content.is_empty() {
        ui.centered_and_justified(|ui| ui.label(RichText::new("No changes to show").weak()));
        return;
    }
    let row_height = ui.text_style_height(&TextStyle::Monospace);
    let width = ui.available_width();
    egui::ScrollArea::vertical().id_salt(("diff", &content.title)).auto_shrink(false).show_rows(
        ui,
        row_height,
        if split { content.split.len() } else { content.lines.len() },
        |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for index in range {
                if split {
                    match content.split[index] {
                        SplitRow::Banner(line_index) => line(ui, content.lines.get(line_index), false, width),
                        SplitRow::Pair { left, right } => {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                let half = (width - 2.0) / 2.0;
                                line(ui, left.and_then(|i| content.lines.get(i)).map(|l| side(l, true)).as_ref(), true, half);
                                line(ui, right.and_then(|i| content.lines.get(i)).map(|l| side(l, false)).as_ref(), true, half);
                            });
                        }
                    }
                } else {
                    line(ui, content.lines.get(index), true, width);
                }
            }
        },
    );
}

/// A line as shown on one side of a split diff, numbered for that side only.
fn side(line: &DiffLine, left: bool) -> DiffLine {
    let mut copy = line.clone();
    if line.kind == DiffLineKind::Context {
        if left {
            copy.new_number = None
        } else {
            copy.old_number = None
        }
    }
    copy
}
