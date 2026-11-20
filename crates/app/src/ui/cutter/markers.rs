//! Named selections ("Marker") of the sample cutter: set with M, listed under the waveform,
//! jump / rename / delete, and saved all at once (Shift+M) through the normal save path.

use super::*;

const MARKER_COLOR: Color32 = Color32::from_rgb(120, 220, 160);

/// A stored selection of the loaded file.
#[derive(Clone, Debug, PartialEq)]
pub struct CutMarker {
    pub name: String,
    pub sel: (usize, usize),
}

/// First free name "Marker N" (N counts from 1).
pub(super) fn next_marker_name(markers: &[CutMarker]) -> String {
    (1..).map(|n| format!("Marker {n}")).find(|n| !markers.iter().any(|m| m.name == *n)).unwrap_or_default()
}

/// Index of the marker before / after position `pos` (by start), ordered by start.
pub(super) fn marker_step(markers: &[CutMarker], pos: usize, forward: bool) -> Option<usize> {
    let it = markers.iter().enumerate();
    if forward {
        it.filter(|(_, m)| m.sel.0 > pos).min_by_key(|(_, m)| m.sel.0).map(|(i, _)| i)
    } else {
        it.filter(|(_, m)| m.sel.0 < pos).max_by_key(|(_, m)| m.sel.0).map(|(i, _)| i)
    }
}

/// File-name safe label of a marker (used as part of the saved file name).
pub(super) fn marker_label(name: &str) -> String {
    let n = name.trim();
    if n.is_empty() {
        "Marker".into()
    } else {
        n.to_string()
    }
}

impl Trim {
    /// Stores the current selection as a marker; an identical one is not added twice.
    /// Returns the marker's index.
    pub(super) fn add_marker(&mut self) -> usize {
        if let Some(i) = self.markers.iter().position(|m| m.sel == self.sel) {
            return i;
        }
        let name = next_marker_name(&self.markers);
        self.markers.push(CutMarker { name, sel: self.sel });
        self.markers.len() - 1
    }

    /// Selects marker `i` and scrolls it into view.
    pub(super) fn jump_marker(&mut self, i: usize) {
        if let Some(m) = self.markers.get(i) {
            let len = self.data.as_ref().map(|d| d.len()).unwrap_or(m.sel.1);
            self.sel = (m.sel.0.min(len), m.sel.1.min(len).max(m.sel.0.min(len) + 1));
            self.reveal_sel();
        }
    }
}

/// Marker ranges as thin bars with their names along the top of the waveform.
pub(super) fn paint_markers(p: &egui::Painter, rect: Rect, markers: &[CutMarker], x_of: &dyn Fn(f64) -> f32) {
    for m in markers {
        let (a, b) = (x_of(m.sel.0 as f64), x_of(m.sel.1 as f64));
        if b < rect.left() || a > rect.right() {
            continue;
        }
        let (a, b) = (a.max(rect.left()), b.min(rect.right()).max(a.max(rect.left()) + 1.0));
        let bar = Rect::from_min_max(pos2(a, rect.top() + 10.0), pos2(b, rect.top() + 13.0));
        p.rect_filled(bar, 1.0, MARKER_COLOR.gamma_multiply(0.8));
        p.text(pos2(a + 2.0, rect.top() + 14.0), Align2::LEFT_TOP, &m.name, FontId::proportional(9.0), MARKER_COLOR);
    }
}

/// "Marker" row: add, list with jump / rename / delete, save all.
pub(super) fn markers_panel(app: &mut TStudio, ui: &mut Ui, acts: &mut Vec<Act>) {
    let sr = app.fetch.trim.sr();
    let t = &mut app.fetch.trim;
    if t.data.is_none() {
        return;
    }
    let mut jump: Option<usize> = None;
    let mut remove: Option<usize> = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Marker").strong());
        if ui.button("+ Auswahl merken").on_hover_text("M – aktuelle Auswahl als Marker speichern").clicked() {
            t.add_marker();
        }
        if ui
            .add_enabled(!t.markers.is_empty(), egui::Button::new(format!("💾 Alle markierten speichern ({})", t.markers.len())))
            .on_hover_text(format!("Shift+M – jeden Marker als eigene Datei in {} speichern", CATEGORIES[t.category]))
            .clicked()
        {
            acts.push(Act::SaveMarkers);
        }
        if !t.markers.is_empty() && ui.small_button("Alle löschen").clicked() {
            t.markers.clear();
            t.marker_edit = None;
        }
        if t.markers.is_empty() {
            ui.weak("M setzt einen Marker auf die Auswahl");
        }
    });
    for (i, m) in t.markers.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let current = t.sel == m.sel;
            if ui.selectable_label(current, "▶").on_hover_text("Hinspringen und abspielen").clicked() {
                jump = Some(i);
            }
            if t.marker_edit == Some(i) {
                let r = ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(140.0));
                if r.lost_focus() {
                    t.marker_edit = None;
                } else {
                    r.request_focus();
                }
            } else {
                let r = ui.add(egui::Label::new(RichText::new(&m.name).color(MARKER_COLOR)).sense(Sense::click())).on_hover_text("Klick = hinspringen · Doppelklick = umbenennen");
                if r.double_clicked() {
                    t.marker_edit = Some(i);
                } else if r.clicked() {
                    jump = Some(i);
                }
            }
            let (a, b) = (m.sel.0 as f64 / sr, m.sel.1 as f64 / sr);
            ui.label(RichText::new(format!("{a:.3}–{b:.3} s ({:.3} s)", b - a)).monospace().small().color(TEXT_DIM));
            if ui.small_button("Auswahl übernehmen").on_hover_text("Marker auf die aktuelle Auswahl setzen").clicked() {
                m.sel = t.sel;
            }
            if ui.small_button("✕").on_hover_text("Marker löschen").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        t.markers.remove(i);
        t.marker_edit = None;
    }
    if let Some(i) = jump {
        t.jump_marker(i);
        acts.push(Act::PlaySel);
    }
}
