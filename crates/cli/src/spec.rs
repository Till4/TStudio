//! Small text grammars: note names, note lists and step strings.

use tstudio_project::PPQ;

/// "C4" / "F#3" / "Bb2" / "60" → MIDI key (C4 = 60, like the GUI).
pub fn parse_key(s: &str) -> Result<u8, String> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i32>() {
        return u8::try_from(n).ok().filter(|k| *k <= 127).ok_or_else(|| format!("Taste außerhalb 0..127: {s}"));
    }
    let mut chars = s.chars().peekable();
    let letter = chars.next().ok_or("leere Note")?.to_ascii_uppercase();
    let base = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' | 'H' => 11,
        _ => return Err(format!("unbekannte Note: {s}")),
    };
    let mut acc = 0;
    while let Some(&c) = chars.peek() {
        match c {
            '#' | '♯' => acc += 1,
            'b' | '♭' => acc -= 1,
            _ => break,
        }
        chars.next();
    }
    let oct: String = chars.collect();
    let oct: i32 = oct.parse().map_err(|_| format!("Oktave fehlt/ungültig in '{s}' (z.B. C4)"))?;
    let key = (oct + 1) * 12 + base + acc;
    u8::try_from(key).ok().filter(|k| *k <= 127).ok_or_else(|| format!("Taste außerhalb 0..127: {s}"))
}

/// Beats (float or fraction like "1/4") → ticks.
pub fn parse_beats(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let v = if let Some((a, b)) = s.split_once('/') {
        let a: f64 = a.trim().parse().map_err(|_| format!("ungültige Zahl: {s}"))?;
        let b: f64 = b.trim().parse().map_err(|_| format!("ungültige Zahl: {s}"))?;
        if b == 0.0 {
            return Err(format!("Division durch 0: {s}"));
        }
        a / b
    } else {
        s.parse::<f64>().map_err(|_| format!("ungültige Zahl: {s}"))?
    };
    Ok((v * PPQ as f64).round() as i64)
}

pub struct NoteSpec {
    pub keys: Vec<u8>,
    pub start: i64,
    pub len: i64,
    pub velocity: f32,
}

/// `KEY[|KEY…]@START[+LEN][*VEL]` – positions in beats, e.g. `C4@0+1`, `C4|E4|G4@2+2*0.6`, `36@0.5`.
pub fn parse_note(tok: &str, default_len: i64, default_vel: f32) -> Result<NoteSpec, String> {
    let (keys, rest) = tok.split_once('@').ok_or_else(|| format!("'{tok}': Format KEY@START[+LEN][*VEL]"))?;
    let keys = keys.split('|').map(parse_key).collect::<Result<Vec<_>, _>>()?;
    let (rest, vel) = match rest.split_once('*') {
        Some((r, v)) => (r, v.trim().parse::<f32>().map_err(|_| format!("'{tok}': ungültige Velocity"))?),
        None => (rest, default_vel),
    };
    let (start, len) = match rest.split_once('+') {
        Some((s, l)) => (parse_beats(s)?, parse_beats(l)?),
        None => (parse_beats(rest)?, default_len),
    };
    if start < 0 {
        return Err(format!("'{tok}': Start < 0"));
    }
    Ok(NoteSpec { keys, start, len: len.max(1), velocity: vel.clamp(0.0, 1.0) })
}

/// Step string over 16ths: `x` = 0.8, `X` = 1.0, `o` = 0.45 (ghost), `.`/`-`/`_` = rest,
/// spaces and `|` are ignored. Returns (step index, velocity).
pub fn parse_steps(s: &str) -> Result<Vec<(usize, f32)>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    for c in s.chars() {
        let v = match c {
            ' ' | '|' => continue,
            '.' | '-' | '_' | '0' => None,
            'x' | '1' => Some(0.8),
            'X' => Some(1.0),
            'o' => Some(0.45),
            _ => return Err(format!("unbekanntes Step-Zeichen '{c}' (erlaubt: x X o . -)")),
        };
        if let Some(v) = v {
            out.push((i, v));
        }
        i += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(parse_key("C4").unwrap(), 60);
        assert_eq!(parse_key("A4").unwrap(), 69);
        assert_eq!(parse_key("F#3").unwrap(), 54);
        assert_eq!(parse_key("Bb2").unwrap(), 46);
        assert_eq!(parse_key("C-1").unwrap(), 0);
        assert_eq!(parse_key("36").unwrap(), 36);
        assert!(parse_key("X4").is_err());
    }

    #[test]
    fn notes() {
        let n = parse_note("C4|E4@1/2+2*0.5", PPQ, 0.8).unwrap();
        assert_eq!(n.keys, vec![60, 64]);
        assert_eq!(n.start, PPQ / 2);
        assert_eq!(n.len, 2 * PPQ);
        assert_eq!(n.velocity, 0.5);
        let n = parse_note("36@3", PPQ / 4, 0.8).unwrap();
        assert_eq!((n.start, n.len), (3 * PPQ, PPQ / 4));
    }

    #[test]
    fn steps() {
        let s = parse_steps("x... X.o. | x-x-").unwrap();
        assert_eq!(s, vec![(0, 0.8), (4, 1.0), (6, 0.45), (8, 0.8), (10, 0.8)]);
    }
}
