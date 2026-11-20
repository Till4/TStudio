//! Keyboard handling of the sample cutter (keys are listed in the module header of `cutter.rs`).

use super::*;

/// Applies the cutter's keys to `t`; `bpm` is the project tempo (nudge grid fallback).
pub(super) fn handle_keys(ui: &Ui, t: &mut Trim, len: usize, bpm: f64, now: f64, acts: &mut Vec<Act>) {
    if ui.ctx().wants_keyboard_input() {
        return;
    }
    let (keys, shift, ctrl) = ui.input(|i| {
        let k = [
            Key::Space, Key::Enter, Key::ArrowLeft, Key::ArrowRight, Key::L, Key::I, Key::Z, Key::H, Key::Escape, Key::A, Key::M, Key::Comma, Key::Period,
        ]
        .map(|k| i.key_pressed(k));
        (k, i.modifiers.shift, i.modifiers.command)
    });
    let nums = ui.input(|i| [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9].map(|k| i.key_pressed(k)));
    let [space, enter, left, right, l, iso, z, h, esc, a, m, comma, period] = keys;
    if space {
        acts.push(if t.playing.is_some() { Act::Stop } else { Act::PlaySel });
    }
    if esc {
        acts.push(Act::Stop);
    }
    if enter && !ctrl {
        acts.push(Act::Save(Some(t.category)));
    }
    for (k, pressed) in nums.iter().enumerate() {
        if *pressed && !ctrl {
            t.category = k;
            acts.push(Act::Save(Some(k)));
        }
    }
    // Ctrl+←/→ nudges the whole window (one beat, with Shift a fine step)
    if (left || right) && ctrl {
        let step = if shift { fine_step(t.sr()) } else { beat_step(t.sel, t.beats, len, t.sr(), bpm) };
        let delta = if right { step as i64 } else { -(step as i64) };
        let moved = nudged_selection(t.sel, delta, len);
        if moved != t.sel {
            t.sel = moved;
            t.reveal_sel();
            acts.push(Act::PlaySel);
        }
    }
    if right && !shift && !ctrl {
        if let Some(n) = t.next_hit_after(t.sel.0) {
            t.sel = (n, t.next_hit_after(n).unwrap_or(len));
            t.reveal_sel();
            acts.push(Act::PlaySel);
        }
    }
    if left && !shift && !ctrl {
        if let Some(n) = t.prev_hit_before(t.sel.0) {
            t.sel = (n, t.next_hit_after(n).unwrap_or(len));
            t.reveal_sel();
            acts.push(Act::PlaySel);
        }
    }
    if right && shift && !ctrl {
        t.sel.1 = t.next_hit_after(t.sel.1).unwrap_or(len);
        t.reveal_sel();
        acts.push(Act::PlaySel);
    }
    if left && shift && !ctrl {
        if let Some(n) = t.prev_hit_before(t.sel.1).filter(|n| *n > t.sel.0) {
            t.sel.1 = n;
            acts.push(Act::PlaySel);
        }
    }
    if l {
        t.loop_play = !t.loop_play;
        if t.playing.is_some() {
            acts.push(Act::PlaySel);
        }
    }
    if iso {
        t.iso_on = !t.iso_on;
        t.iso_changed = Some(now);
        if t.playing.is_some() {
            acts.push(Act::PlaySel);
        }
    }
    if z && !shift && !ctrl {
        let pad = ((t.sel.1 - t.sel.0) as f64 * 0.08).max(32.0);
        t.view = ((t.sel.0 as f64 - pad).max(0.0), (t.sel.1 as f64 + pad).min(len as f64));
    }
    if z && shift && !ctrl {
        t.view = (0.0, len as f64);
    }
    if h {
        acts.push(Act::SaveHits);
    }
    if a && ctrl {
        t.sel = (0, len);
    }
    // markers: M add, Shift+M save all, , / . previous / next
    if m && !ctrl {
        if shift {
            acts.push(Act::SaveMarkers);
        } else {
            t.add_marker();
        }
    }
    if comma || period {
        if let Some(i) = marker_step(&t.markers, t.sel.0, period) {
            t.jump_marker(i);
            acts.push(Act::PlaySel);
        }
    }
}
