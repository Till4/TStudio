//! One polyphonic voice: oscillators, filters, envelopes, LFOs and modulation evaluation.

use super::*;

#[inline]
pub(super) fn warp_param(mode: WarpMode, a: f32) -> f32 {
    let a = a.clamp(0.0, 1.0);
    match mode {
        WarpMode::BendPlus => 1.0 + 8.0 * a,
        WarpMode::BendMinus => 1.0 / (1.0 + 8.0 * a),
        WarpMode::Sync => 1.0 + 7.0 * a,
        WarpMode::Pwm => 1.0 / (1.0 - 0.95 * a),
        WarpMode::Quantize => 2f32.powf(1.0 + 9.0 * (1.0 - a)).round(),
        _ => 0.0,
    }
}

/// Warped phase. `k` from `warp_param`, `a` = amount, `fm` = modulator sample.
#[inline]
pub(super) fn warp_phase(mode: WarpMode, k: f32, a: f32, p: f32, fm: f32) -> f32 {
    match mode {
        WarpMode::Off => p,
        WarpMode::BendPlus | WarpMode::BendMinus => p * k / (1.0 + p * (k - 1.0)),
        WarpMode::Sync => {
            let q = p * k;
            q - q.floor()
        }
        WarpMode::Pwm => {
            let q = p * k;
            if q < 1.0 {
                q
            } else {
                0.0
            }
        }
        WarpMode::Quantize => (p * k).floor() / k,
        WarpMode::FmOsc1 | WarpMode::FmOsc2 | WarpMode::FmOsc3 | WarpMode::FmSub => {
            let q = p + a.clamp(0.0, 1.0) * 1.5 * fm;
            q - q.floor()
        }
    }
}

/// how much a warp widens the spectrum (for the mip choice)
#[inline]
pub(super) fn warp_bandwidth(mode: WarpMode, k: f32, a: f32) -> f32 {
    match mode {
        WarpMode::Sync | WarpMode::Pwm => k,
        WarpMode::BendPlus => k.sqrt(),
        WarpMode::BendMinus => (1.0 / k).sqrt(),
        WarpMode::FmOsc1 | WarpMode::FmOsc2 | WarpMode::FmOsc3 | WarpMode::FmSub => 1.0 + 6.0 * a,
        _ => 1.0,
    }
}

#[inline]
pub(super) fn start_phase(op: &OscParams, r: f32, kick: bool) -> f32 {
    if kick {
        return op.phase.rem_euclid(1.0);
    }
    let ph = op.phase + op.rand_phase * r;
    if ph > 1.0 {
        ph - ph.floor()
    } else {
        ph
    }
}

#[derive(Clone)]
pub(super) struct OscState {
    pub(super) osc: [Osc; MAX_UNISON],
    pub(super) dt: [f32; MAX_UNISON],
    /// per unison voice (left, right) gain
    pub(super) g: [(f32, f32); MAX_UNISON],
    pub(super) n: usize,
    pub(super) mip: usize,
    pub(super) pos: f32,
    pub(super) warp_k: f32,
    pub(super) warp_a: f32,
    /// last raw sample of the centre voice (FM source)
    pub(super) last: f32,
}

impl Default for OscState {
    fn default() -> Self {
        Self { osc: [Osc::default(); MAX_UNISON], dt: [0.0; MAX_UNISON], g: [(0.0, 0.0); MAX_UNISON], n: 1, mip: 0, pos: 0.0, warp_k: 0.0, warp_a: 0.0, last: 0.0 }
    }
}

#[derive(Clone)]
pub(super) struct SynthVoice {
    pub(super) active: bool,
    pub(super) key: u8,
    pub(super) vel: f32,
    pub(super) vel_gain: f32,
    pub(super) age: u64,
    pub(super) pitch: f32,
    pub(super) target_pitch: f32,
    pub(super) oscs: [OscState; NUM_OSC],
    pub(super) sub: Osc,
    pub(super) sub_last: f32,
    pub(super) lfo: [Osc; NUM_LFO],
    pub(super) lfo_val: [f32; NUM_LFO],
    pub(super) lfo_sh: [f32; NUM_LFO],
    /// rng of the original voice (start phases of the first 7 unison slots, noise)
    pub(super) rng: Rng,
    /// rng for everything added later (keeps the original sequence intact)
    pub(super) rng2: Rng,
    pub(super) flt1: [SynthFilter; 2],
    pub(super) flt2: [SynthFilter; 2],
    pub(super) amp_env: Adsr,
    pub(super) flt_env: Adsr,
    pub(super) mod_env: Adsr,
    pub(super) remaining: Option<u64>,
    pub(super) held: bool,
    pub(super) ctr: u32,
    // control-rate state
    pub(super) dt_sub: f32,
    pub(super) sub_gain: f32,
    pub(super) noise_gain: f32,
    pub(super) amp_mod: f32,
    pub(super) pan_l: f32,
    pub(super) pan_r: f32,
    pub(super) flt_env_val: f32,
    /// seconds since note start (pitch envelopes)
    pub(super) t_since: f32,
    pub(super) random: f32,
    pub(super) mods: [f32; ModDest::COUNT],
    pub(super) f1_drive: f32,
    pub(super) f1_norm: f32,
    pub(super) f2_drive: f32,
    pub(super) f2_norm: f32,
    // noise
    pub(super) noise_lp: OnePole,
    pub(super) noise_hold: f32,
    pub(super) noise_ph: f32,
    // kick
    pub(super) kick_on: bool,
    pub(super) kick_drive: f32,
    pub(super) kick_norm: f32,
    pub(super) click_env: f32,
    pub(super) click_coef: f32,
    pub(super) click_ph: f32,
    pub(super) click_dt: f32,
    pub(super) click_gain: f32,
    pub(super) click_bp: Svf,
    pub(super) body_env: f32,
    pub(super) body_hold: u32,
    pub(super) body_coef: f32,
    pub(super) rumble_ph: f32,
    pub(super) rumble_dt: f32,
    pub(super) rumble_rise: f32,
    pub(super) rumble_env: f32,
    pub(super) rumble_rel: f32,
    pub(super) rumble_delay: u32,
    pub(super) rumble_lp: [OnePole; 2],
    pub(super) rumble_wob: OnePole,
    pub(super) dc: [(f32, f32); 2],
}

impl SynthVoice {
    pub(super) fn new(sr: f32, seed: u32) -> Self {
        let comb = (sr / COMB_MIN_HZ) as usize + 2;
        Self {
            active: false,
            key: 0,
            vel: 1.0,
            vel_gain: 1.0,
            age: 0,
            pitch: 60.0,
            target_pitch: 60.0,
            oscs: [OscState::default(), OscState::default(), OscState::default()],
            sub: Osc::default(),
            sub_last: 0.0,
            lfo: [Osc::default(); NUM_LFO],
            lfo_val: [0.0; NUM_LFO],
            lfo_sh: [0.0; NUM_LFO],
            rng: Rng(0x9E37_79B9 ^ seed.wrapping_mul(2_654_435_761)),
            rng2: Rng(0x85EB_CA6B ^ seed.wrapping_mul(0x27D4_EB2F)),
            flt1: [SynthFilter::new(comb), SynthFilter::new(comb)],
            flt2: [SynthFilter::new(comb), SynthFilter::new(comb)],
            amp_env: Adsr::new(sr),
            flt_env: Adsr::new(sr),
            mod_env: Adsr::new(sr),
            remaining: None,
            held: false,
            ctr: 0,
            dt_sub: 0.0,
            sub_gain: 0.0,
            noise_gain: 0.0,
            amp_mod: 1.0,
            pan_l: 1.0,
            pan_r: 1.0,
            flt_env_val: 0.0,
            t_since: 0.0,
            random: 0.0,
            mods: [0.0; ModDest::COUNT],
            f1_drive: 0.0,
            f1_norm: 1.0,
            f2_drive: 0.0,
            f2_norm: 1.0,
            noise_lp: OnePole::default(),
            noise_hold: 0.0,
            noise_ph: 0.0,
            kick_on: false,
            kick_drive: 0.0,
            kick_norm: 1.0,
            click_env: 0.0,
            click_coef: 0.0,
            click_ph: 0.0,
            click_dt: 0.0,
            click_gain: 0.0,
            click_bp: Svf::default(),
            body_env: 1.0,
            body_hold: 0,
            body_coef: 1.0,
            rumble_ph: 0.0,
            rumble_dt: 0.0,
            rumble_rise: 0.0,
            rumble_env: 1.0,
            rumble_rel: 1.0,
            rumble_delay: 0,
            rumble_lp: [OnePole::default(); 2],
            rumble_wob: OnePole::default(),
            dc: [(0.0, 0.0); 2],
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start(&mut self, p: &SynthParams, sr: f32, key: u8, vel: f32, dur: Option<u64>, from_pitch: f32, age: u64) {
        self.active = true;
        self.key = key;
        self.vel = vel;
        self.vel_gain = 1.0 - p.velocity_sens + p.velocity_sens * vel;
        self.age = age;
        self.kick_on = p.kick.enabled;
        self.target_pitch = if self.kick_on { p.kick.body_note(key) } else { key as f32 };
        self.pitch = if p.glide > 0.0 && !self.kick_on && !(p.mono && p.slide_only) { from_pitch } else { self.target_pitch };
        self.remaining = dur;
        self.held = dur.is_none();
        if self.kick_on && p.amp_env.sustain <= 0.0 {
            // kicks are one-shots: the envelope plays out regardless of the note length
            self.remaining = None;
            self.held = false;
        }
        self.ctr = 0;
        self.t_since = 0.0;
        // random start phases give unison its width. The first 7 slots of osc 1 and 2
        // draw from the original generator in the original order.
        for oi in 0..2 {
            let op = p.osc(oi);
            for u in 0..LEGACY_UNISON {
                let r = self.rng.next_f32() * 0.5 + 0.5;
                self.oscs[oi].osc[u].phase = start_phase(op, r, self.kick_on);
            }
        }
        for oi in 0..NUM_OSC {
            let op = p.osc(oi);
            let from = if oi < 2 { LEGACY_UNISON } else { 0 };
            for u in from..MAX_UNISON {
                let r = self.rng2.next_f32() * 0.5 + 0.5;
                self.oscs[oi].osc[u].phase = start_phase(op, r, self.kick_on);
            }
            self.oscs[oi].last = 0.0;
        }
        self.sub.phase = 0.0;
        self.sub_last = 0.0;
        self.random = self.rng2.next_f32();
        for (i, l) in self.lfo.iter_mut().enumerate() {
            let lp = p.lfo(i);
            if lp.retrig {
                l.phase = lp.phase.rem_euclid(1.0);
                self.lfo_sh[i] = self.rng2.next_f32();
            }
        }
        for f in self.flt1.iter_mut().chain(self.flt2.iter_mut()) {
            f.reset();
        }
        self.noise_lp.z = 0.0;
        self.dc = [(0.0, 0.0); 2];
        // kick layers
        let k = &p.kick;
        self.click_env = 1.0;
        self.click_coef = (-1.0 / (k.click_decay.max(0.0005) * sr)).exp();
        self.click_ph = 0.0;
        self.click_bp.reset();
        self.click_bp.set(k.click_tone, 0.25, sr);
        self.body_env = 1.0;
        self.body_hold = (k.body_hold.max(0.0) * sr) as u32;
        self.body_coef = if k.body_decay > 0.0 { (-7.0 / (k.body_decay * sr)).exp() } else { 1.0 };
        self.rumble_ph = 0.0;
        self.rumble_rise = 0.0;
        self.rumble_env = 1.0;
        self.rumble_rel = (-7.0 / (k.rumble_decay.max(0.01) * sr)).exp();
        self.rumble_delay = (k.rumble_delay.max(0.0) * sr) as u32;
        self.rumble_lp = [OnePole::default(); 2];
        self.rumble_wob.z = 0.0;
        self.amp_env.set_params(&p.amp_env);
        self.flt_env.set_params(&p.filter_env);
        self.mod_env.set_params(&p.mod_env);
        self.amp_env.reset();
        self.flt_env.reset();
        self.mod_env.reset();
        self.amp_env.trigger();
        self.flt_env.trigger();
        self.mod_env.trigger();
    }

    pub(super) fn release(&mut self) {
        self.amp_env.release();
        self.flt_env.release();
        self.mod_env.release();
        self.held = false;
        self.remaining = None;
    }

    pub(super) fn control(&mut self, p: &SynthParams, sr: f32, bpm: f32) {
        let ctl_dt = CONTROL_RATE as f32 / sr;
        // ---- LFOs (advance by one control period)
        let mut pitch_mod = 0.0;
        let mut cutoff_mod = 0.0;
        let mut amp_mod = 1.0;
        let mut pan_mod = 0.0;
        const RATE_DEST: [ModDest; NUM_LFO] = [ModDest::Lfo1Rate, ModDest::Lfo2Rate, ModDest::Lfo3Rate, ModDest::Lfo4Rate];
        for i in 0..NUM_LFO {
            let lp = p.lfo(i);
            let rm = self.mods[RATE_DEST[i] as usize];
            let inc = if !lp.sync && rm == 0.0 {
                // original expression (bit-exact for old projects)
                lp.rate * CONTROL_RATE as f32 / sr
            } else {
                lp.hz(bpm) * 2f32.powf(rm * 4.0) * ctl_dt
            };
            let v = match lp.shape {
                LfoShape::Wave => self.lfo[i].next(lp.wave, inc),
                shape => {
                    let ph = self.lfo[i].phase;
                    let mut np = ph + inc.clamp(0.0, 0.49);
                    if np >= 1.0 {
                        np -= 1.0;
                        self.lfo_sh[i] = self.rng2.next_f32();
                    }
                    self.lfo[i].phase = np;
                    if shape == LfoShape::SampleHold {
                        self.lfo_sh[i]
                    } else {
                        lp.value_at(ph)
                    }
                }
            };
            self.lfo_val[i] = v;
            match lp.target {
                LfoTarget::Off => {}
                LfoTarget::Pitch => pitch_mod += v * lp.amount * 12.0,
                LfoTarget::Cutoff => cutoff_mod += v * lp.amount * 5.0,
                LfoTarget::Amp => amp_mod *= 1.0 - lp.amount * (0.5 - 0.5 * v),
                LfoTarget::Pan => pan_mod += v * lp.amount,
            }
        }

        // ---- mod matrix
        let sweep = if self.kick_on { p.kick.sweep(self.t_since) } else { 0.0 };
        if p.mod_matrix.iter().any(|s| s.active()) {
            let mut m = [0.0f32; ModDest::COUNT];
            for s in p.mod_matrix.iter().filter(|s| s.active() && !s.dest.is_global()) {
                let v = match s.source {
                    ModSource::Off => 0.0,
                    ModSource::AmpEnv => self.amp_env.level,
                    ModSource::FilterEnv => self.flt_env.level,
                    ModSource::ModEnv => self.mod_env.level,
                    ModSource::Lfo1 => self.lfo_val[0],
                    ModSource::Lfo2 => self.lfo_val[1],
                    ModSource::Lfo3 => self.lfo_val[2],
                    ModSource::Lfo4 => self.lfo_val[3],
                    ModSource::Velocity => self.vel,
                    ModSource::Note => ((self.pitch - 60.0) / 48.0).clamp(-1.5, 1.5),
                    ModSource::Random => self.random,
                    ModSource::Macro1 => p.macro1,
                    ModSource::Macro2 => p.macro2,
                    ModSource::Macro3 => p.macro3,
                    ModSource::Macro4 => p.macro4,
                    ModSource::KickSweep => sweep,
                };
                m[s.dest as usize] += v * s.amount;
            }
            self.mods = m;
        } else if self.mods.iter().any(|v| *v != 0.0) {
            self.mods = [0.0; ModDest::COUNT];
        }
        let m = self.mods;
        let md = |d: ModDest| m[d as usize];

        let amp_m = md(ModDest::Amp);
        if amp_m != 0.0 {
            amp_mod *= (1.0 + amp_m).max(0.0);
        }
        self.amp_mod = amp_mod;
        let pan_m = md(ModDest::Pan);
        let (pl, pr) = crate::pan_gains(if pan_m != 0.0 { pan_mod + pan_m } else { pan_mod });
        self.pan_l = pl;
        self.pan_r = pr;

        // ---- glide
        if p.glide > 0.0 && !self.kick_on {
            let coef = 1.0 - (-(CONTROL_RATE as f32) / (p.glide * sr / 4.0)).exp();
            self.pitch += (self.target_pitch - self.pitch) * coef;
        } else {
            self.pitch = self.target_pitch;
        }

        // ---- pitch
        let pitch_env = if p.pitch_env_amount != 0.0 { p.pitch_env_amount * (-self.t_since / p.pitch_env_decay.max(0.001)).exp() } else { 0.0 };
        let kick_env = if self.kick_on { (p.kick.pitch_start + md(ModDest::KickPitch) * 48.0) * sweep } else { 0.0 };
        self.t_since += ctl_dt;
        let mut base = self.pitch + pitch_mod + pitch_env;
        let gp = md(ModDest::Pitch) * 24.0;
        if kick_env != 0.0 || gp != 0.0 {
            base += kick_env + gp;
        }
        const OSC_PITCH: [ModDest; 3] = [ModDest::Osc1Pitch, ModDest::Osc2Pitch, ModDest::Osc3Pitch];
        const OSC_POS: [ModDest; 3] = [ModDest::Osc1Pos, ModDest::Osc2Pos, ModDest::Osc3Pos];
        const OSC_WARP: [ModDest; 3] = [ModDest::Osc1Warp, ModDest::Osc2Warp, ModDest::Osc3Warp];
        const OSC_LVL: [ModDest; 3] = [ModDest::Osc1Level, ModDest::Osc2Level, ModDest::Osc3Level];
        let det_m = md(ModDest::Detune);
        for (oi, st) in self.oscs.iter_mut().enumerate() {
            let osc = p.osc(oi);
            if !osc.enabled {
                continue;
            }
            let n = osc.unison.clamp(1, MAX_UNISON as u32) as usize;
            st.n = n;
            let op = md(OSC_PITCH[oi]) * 24.0;
            let mut k = base + (osc.octave * 12 + osc.semi) as f32 + osc.fine / 100.0;
            if op != 0.0 {
                k += op;
            }
            let detune = if det_m != 0.0 { osc.detune * (1.0 + det_m).max(0.0) } else { osc.detune };
            for (u, dt) in st.dt.iter_mut().enumerate().take(n) {
                let det = if n > 1 { detune / 100.0 * (u as f32 / (n - 1) as f32 - 0.5) } else { 0.0 };
                *dt = midi_to_freq(k + det) / sr;
            }
            st.pos = (osc.position + md(OSC_POS[oi])).clamp(0.0, 1.0);
            st.warp_a = (osc.warp_amount + md(OSC_WARP[oi])).clamp(0.0, 1.0);
            st.warp_k = warp_param(osc.warp, st.warp_a);
            let max_dt = st.dt[..n].iter().fold(0.0f32, |a, b| a.max(*b));
            st.mip = Wavetable::mip_for(max_dt * warp_bandwidth(osc.warp, st.warp_k, st.warp_a));
            // unison gains (blend: centre voices at 1, side voices at `blend`)
            let lm = md(OSC_LVL[oi]);
            let level = if lm != 0.0 { (osc.level + lm).max(0.0) } else { osc.level };
            let blend = osc.blend.clamp(0.0, 1.0);
            let centres = if n % 2 == 1 { 1usize } else { 2 };
            let norm = if blend == 1.0 { level / (n as f32).sqrt() } else { level / (centres as f32 + (n - centres) as f32 * blend * blend).sqrt() };
            for u in 0..n {
                let mut pan = if n > 1 { osc.spread * (2.0 * u as f32 / (n - 1) as f32 - 1.0) } else { 0.0 };
                if osc.pan != 0.0 {
                    pan += osc.pan;
                }
                let (a, b) = crate::pan_gains(pan);
                let centre = n == 1 || (n % 2 == 1 && u == n / 2) || (n % 2 == 0 && (u == n / 2 || u + 1 == n / 2));
                let w = if centre { 1.0 } else { blend };
                st.g[u] = (a * norm * w, b * norm * w);
            }
        }
        self.dt_sub = midi_to_freq(base + (p.osc1.octave * 12 + p.osc1.semi) as f32 + (p.sub_octave * 12) as f32) / sr;
        let sm = md(ModDest::SubLevel);
        self.sub_gain = if sm != 0.0 { (p.sub_level + sm).max(0.0) } else { p.sub_level };
        let nm = md(ModDest::NoiseLevel);
        self.noise_gain = if nm != 0.0 { (p.noise_level + nm).max(0.0) } else { p.noise_level };
        if p.noise_tone < 19000.0 {
            self.noise_lp.set(p.noise_tone, sr);
        }

        // ---- kick
        if self.kick_on {
            let k = &p.kick;
            let d = k.drive_db + md(ModDest::KickDrive) * 24.0;
            self.kick_drive = if d > 0.0 { crate::db_to_gain(d) } else { 0.0 };
            self.kick_norm = k.drive_mode.norm(self.kick_drive);
            self.click_gain = (k.click_level + md(ModDest::ClickLevel)).max(0.0);
            let chirp = if k.click_type == ClickType::Snap { 1.0 + 3.0 * self.click_env } else { 1.0 };
            self.click_dt = k.click_tone * chirp / sr;
            self.rumble_dt = midi_to_freq(self.target_pitch) / sr;
            self.rumble_lp[0].set(k.rumble_tone, sr);
            self.rumble_lp[1].set(k.rumble_tone, sr);
            self.rumble_wob.set(18.0, sr);
        }

        // ---- filters
        let key_oct = (self.pitch - 60.0) / 12.0 * p.key_track;
        let mut oct = key_oct + self.flt_env_val * p.filter_env_amount + cutoff_mod;
        let c1 = md(ModDest::Cutoff1);
        if c1 != 0.0 {
            oct += c1 * 6.0;
        }
        let fc = p.cutoff * 2f32.powf(oct);
        let r1 = md(ModDest::Reso1);
        let res1 = if r1 != 0.0 { (p.resonance + r1).clamp(0.0, 1.0) } else { p.resonance };
        self.flt1[0].set(p.filter_type, fc, res1, sr);
        self.flt1[1].set(p.filter_type, fc, res1, sr);
        let fdm = md(ModDest::FilterDrive) * 24.0;
        let fd1 = p.filter_drive + fdm;
        self.f1_drive = if fd1 > 0.0 { crate::db_to_gain(fd1) } else { 0.0 };
        self.f1_norm = 1.0 / self.f1_drive.max(1e-3).tanh();
        if p.filter2.enabled {
            let f2 = &p.filter2;
            let oct2 = (self.pitch - 60.0) / 12.0 * f2.key_track + self.flt_env_val * f2.env_amount + cutoff_mod + md(ModDest::Cutoff2) * 6.0;
            let fc2 = f2.cutoff * 2f32.powf(oct2);
            let res2 = (f2.resonance + md(ModDest::Reso2)).clamp(0.0, 1.0);
            self.flt2[0].set(f2.kind, fc2, res2, sr);
            self.flt2[1].set(f2.kind, fc2, res2, sr);
            let fd2 = f2.drive_db + fdm;
            self.f2_drive = if fd2 > 0.0 { crate::db_to_gain(fd2) } else { 0.0 };
            self.f2_norm = 1.0 / self.f2_drive.max(1e-3).tanh();
        }
    }

    pub(super) fn render(&mut self, p: &SynthParams, sr: f32, bpm: f32, l: &mut [f32], r: &mut [f32]) {
        self.amp_env.set_params(&p.amp_env);
        self.flt_env.set_params(&p.filter_env);
        self.mod_env.set_params(&p.mod_env);
        let drive = if p.drive_db > 0.0 { crate::db_to_gain(p.drive_db) } else { 0.0 };
        let classic_drive = p.drive_mode == DriveMode::Soft;
        // classic expression kept for old projects
        let drive_norm = if drive > 0.0 {
            if classic_drive {
                1.0 / drive.tanh()
            } else {
                p.drive_mode.norm(drive)
            }
        } else {
            1.0
        };
        let tables = [wavetable::table(p.osc1.table), wavetable::table(p.osc2.table), wavetable::table(p.osc3.table)];
        let fm_used = [p.osc1.warp.fm_source(), p.osc2.warp.fm_source(), p.osc3.warp.fm_source()];
        let sub_needed = fm_used.contains(&Some(3));
        let k = &p.kick;
        let f2_on = p.filter2.enabled;

        for i in 0..l.len() {
            if self.ctr % CONTROL_RATE == 0 {
                self.control(p, sr, bpm);
            }
            self.ctr = self.ctr.wrapping_add(1);

            let mut sl = 0.0;
            let mut sr_ = 0.0;
            for oi in 0..NUM_OSC {
                let op = p.osc(oi);
                if !op.enabled {
                    continue;
                }
                let fm = match fm_used[oi] {
                    Some(3) => self.sub_last,
                    Some(j) => self.oscs[j].last,
                    None => 0.0,
                };
                let st = &mut self.oscs[oi];
                let n = st.n;
                let centre = n / 2;
                let classic = op.table == WtKind::Basic && op.warp == WarpMode::Off;
                let tbl = tables[oi];
                let frame = op.basic_frame();
                for u in 0..n {
                    let s = if classic {
                        st.osc[u].next(op.wave, st.dt[u])
                    } else {
                        let o = &mut st.osc[u];
                        let q = warp_phase(op.warp, st.warp_k, st.warp_a, o.phase, fm);
                        let v = if op.table == WtKind::Basic { tbl.sample_frame(frame, q, st.mip) } else { tbl.sample(st.pos, q, st.mip) };
                        o.phase += st.dt[u].clamp(0.0, 0.49);
                        if o.phase >= 1.0 {
                            o.phase -= 1.0;
                        }
                        v
                    };
                    if u == centre {
                        st.last = s;
                    }
                    sl += s * st.g[u].0;
                    sr_ += s * st.g[u].1;
                }
            }
            if self.sub_gain > 0.0 || sub_needed {
                let raw = self.sub.next(p.sub_wave, self.dt_sub);
                self.sub_last = raw;
                let s = raw * self.sub_gain;
                sl += s;
                sr_ += s;
            }
            // kick: without its own body envelope the amp envelope shapes the body *before* the
            // drive, so the distortion holds the decaying tail up (fat, long bass) instead of the
            // envelope fading a constant distorted tone
            let amp_v = self.amp_env.next();
            let env_pre = self.kick_on && k.body_decay <= 0.0;
            if self.kick_on && k.body_decay > 0.0 {
                if self.body_hold > 0 {
                    self.body_hold -= 1;
                } else {
                    self.body_env *= self.body_coef;
                }
                sl *= self.body_env;
                sr_ *= self.body_env;
            }
            if self.noise_gain > 0.0 {
                let raw = match p.noise_type {
                    NoiseType::White => self.rng.next_f32(),
                    NoiseType::Crackle => {
                        let v = self.rng.next_f32();
                        if v.abs() > 0.97 {
                            v.signum()
                        } else {
                            0.0
                        }
                    }
                    NoiseType::Digital => {
                        self.noise_ph += self.oscs[0].dt[0] * 4.0;
                        if self.noise_ph >= 1.0 {
                            self.noise_ph -= self.noise_ph.floor();
                            self.noise_hold = self.rng.next_f32();
                        }
                        self.noise_hold
                    }
                };
                let raw = if p.noise_tone < 19000.0 { self.noise_lp.process(raw) } else { raw };
                let s = raw * self.noise_gain;
                sl += s;
                sr_ += s;
            }
            if self.kick_on {
                if env_pre {
                    sl *= amp_v;
                    sr_ *= amp_v;
                }
                // click / transient
                if self.click_gain > 0.0 && self.click_env > 1e-4 {
                    let c = match k.click_type {
                        ClickType::Noise => self.click_bp.process(self.rng2.next_f32(), FilterMode::BandPass) * 2.5,
                        ClickType::Tick | ClickType::Snap => {
                            self.click_ph += self.click_dt;
                            if self.click_ph >= 1.0 {
                                self.click_ph -= 1.0;
                            }
                            (TAU * self.click_ph).sin()
                        }
                    };
                    let s = c * self.click_env * self.click_gain;
                    self.click_env *= self.click_coef;
                    sl += s;
                    sr_ += s;
                }
                // drive
                if self.kick_drive > 0.0 {
                    sl = k.drive_mode.apply(sl, self.kick_drive, self.kick_norm);
                    sr_ = k.drive_mode.apply(sr_, self.kick_drive, self.kick_norm);
                    if k.drive_mode == DriveMode::Asym {
                        for (x, d) in [(&mut sl, 0usize), (&mut sr_, 1)] {
                            let (x1, y1) = self.dc[d];
                            let y = *x - x1 + 0.995 * y1;
                            self.dc[d] = (*x, y);
                            *x = y;
                        }
                    }
                }
                // rumble: delayed, wobbling, saturated drone at the body pitch
                if k.rumble_level > 0.0 {
                    if self.rumble_delay > 0 {
                        self.rumble_delay -= 1;
                    } else {
                        self.rumble_rise += (1.0 - self.rumble_rise) * 0.003;
                        self.rumble_env *= self.rumble_rel;
                    }
                    self.rumble_ph += self.rumble_dt;
                    if self.rumble_ph >= 1.0 {
                        self.rumble_ph -= 1.0;
                    }
                    let wob = self.rumble_wob.process(self.rng2.next_f32());
                    let x = (TAU * self.rumble_ph).sin() * (1.0 + 12.0 * wob) * self.rumble_rise * self.rumble_env * k.rumble_level;
                    let y0 = self.rumble_lp[0].process((x * 4.0).tanh());
                    let y = self.rumble_lp[1].process(y0);
                    sl += y * 1.5;
                    sr_ += y * 1.5;
                }
            }
            // filters
            let (mut fl, mut fr) = if p.filter_enabled {
                let (il, ir) = if self.f1_drive > 0.0 { ((sl * self.f1_drive).tanh() * self.f1_norm, (sr_ * self.f1_drive).tanh() * self.f1_norm) } else { (sl, sr_) };
                (self.flt1[0].process(il, p.filter_type, p.filter_mode), self.flt1[1].process(ir, p.filter_type, p.filter_mode))
            } else {
                (sl, sr_)
            };
            if f2_on {
                let f2 = &p.filter2;
                let parallel = p.filter_routing == FilterRouting::Parallel && p.filter_enabled;
                let (il, ir) = if parallel { (sl, sr_) } else { (fl, fr) };
                let (il, ir) = if self.f2_drive > 0.0 { ((il * self.f2_drive).tanh() * self.f2_norm, (ir * self.f2_drive).tanh() * self.f2_norm) } else { (il, ir) };
                let ol = self.flt2[0].process(il, f2.kind, f2.mode);
                let or = self.flt2[1].process(ir, f2.kind, f2.mode);
                if parallel {
                    fl += ol;
                    fr += or;
                } else {
                    fl = ol;
                    fr = or;
                }
            }
            if drive > 0.0 {
                if classic_drive {
                    fl = (fl * drive).tanh() * drive_norm;
                    fr = (fr * drive).tanh() * drive_norm;
                } else {
                    fl = p.drive_mode.apply(fl, drive, drive_norm);
                    fr = p.drive_mode.apply(fr, drive, drive_norm);
                }
            }
            self.flt_env_val = self.flt_env.next();
            self.mod_env.next();
            let a = if env_pre { 1.0 } else { amp_v } * self.vel_gain * self.amp_mod * p.gain;
            l[i] += fl * a * self.pan_l;
            r[i] += fr * a * self.pan_r;

            if let Some(rem) = self.remaining.as_mut() {
                if *rem == 0 {
                    self.release();
                } else {
                    *rem -= 1;
                }
            }
            if !self.amp_env.is_active() {
                self.active = false;
                return;
            }
        }
    }
}
