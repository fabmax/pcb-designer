//! Values with SI prefixes, as users type them and as the app shows them.
//!
//! [`parse`] reads `10k`, `4k7`, `2R2`, `5V1`, `100n`, `10µ` / `10u`, `1M` (mega, as users
//! mean it; SPICE would read milli), `1meg`, `1e-3`, `10 kΩ`, `100nF 50V` (the first value,
//! with an optional unit; what follows a space is ignored). [`format`] writes engineering
//! notation with a prefix: `4.7 kΩ`, `1 µs`. [`spice`] writes a plain exponent for
//! netlists, never a SPICE suffix.

/// Prefix letters and their powers of ten. `M` is mega and `m` milli; `u`, `µ` (micro
/// sign) and `μ` (Greek mu) are micro.
fn prefix(c: char) -> Option<i32> {
    Some(match c {
        'f' => -15,
        'p' => -12,
        'n' => -9,
        'u' | 'µ' | 'μ' => -6,
        'm' => -3,
        'k' | 'K' => 3,
        'M' => 6,
        'G' => 9,
        'T' => 12,
        _ => return None,
    })
}

/// Unit words allowed after a value (case-sensitive except `ohm`).
const UNITS: &[&str] = &["Ω", "ohm", "Ohm", "ohms", "R", "F", "H", "V", "A", "Hz", "s", "W", "%", "°C", "deg", "°"];

/// Letters that stand for the decimal point between digits besides the prefixes:
/// `2R2` (2.2 Ω), `5V1` (5.1 V), `4F7`.
fn decimal_unit(c: char) -> bool {
    matches!(c, 'R' | 'r' | 'V' | 'F' | 'H' | 'A' | 'Ω')
}

/// The value of `text`, or `None` when it doesn't start with one.
pub fn parse(text: &str) -> Option<f64> {
    let b: Vec<char> = text.trim().chars().collect();
    let mut i = 0;
    // The number: sign, digits with at most one point, an exponent.
    let mut num = String::new();
    if matches!(b.first(), Some('+' | '-')) {
        num.push(b[0]);
        i = 1;
    }
    let mut dot = false;
    let mut digits = false;
    while let Some(&c) = b.get(i) {
        if c.is_ascii_digit() {
            digits = true;
        } else if c != '.' || dot {
            break;
        }
        dot |= c == '.';
        num.push(c);
        i += 1;
    }
    if !digits {
        return None;
    }
    if matches!(b.get(i), Some('e' | 'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some('+' | '-')) {
            j += 1;
        }
        if b.get(j).is_some_and(|c| c.is_ascii_digit()) {
            while b.get(j).is_some_and(|c| c.is_ascii_digit()) {
                j += 1;
            }
            num.extend(&b[i..j]);
            i = j;
        }
    }
    let mut value: f64 = num.parse().ok()?;
    while b.get(i) == Some(&' ') {
        i += 1;
    }
    // The multiplier: `meg`, a prefix letter, or a unit letter used as the decimal point.
    let rest: String = b[i..].iter().collect();
    let mut power = 0;
    let digit_at = |k: usize| b.get(k).is_some_and(|c| c.is_ascii_digit());
    if rest.len() >= 3 && rest.is_char_boundary(3) && rest[..3].eq_ignore_ascii_case("meg") {
        power = 6;
        i += 3;
    } else if let Some(p) = b.get(i).and_then(|&c| prefix(c)) {
        power = p;
        i += 1;
    } else if b.get(i).is_some_and(|&c| decimal_unit(c)) && digit_at(i + 1) {
        i += 1;
    }
    // `4k7`, `2R2`, `5V1`: digits after the letter are the fraction.
    if i > 0 && digit_at(i) && !b[i - 1].is_ascii_digit() && b[i - 1] != ' ' {
        if dot {
            return None;
        }
        let mut frac = String::from("0.");
        while digit_at(i) {
            frac.push(b[i]);
            i += 1;
        }
        let frac: f64 = frac.parse().ok()?;
        value += if value.is_sign_negative() { -frac } else { frac };
    }
    let value = value * 10f64.powi(power);
    // What's left: nothing, or a unit; then nothing or a space and anything.
    let tail: String = b[i..].iter().collect();
    let tail_ok = |t: &str| t.is_empty() || t.starts_with(' ');
    if tail_ok(&tail) || UNITS.iter().any(|u| tail.strip_prefix(u).is_some_and(tail_ok)) { Some(value) } else { None }
}

/// `value` in engineering notation with an SI prefix and `unit`: `4.7 kΩ`, `0 V`,
/// `-12 V`, `1.5 ms`. Up to four significant digits, trailing zeros dropped.
pub fn format(value: f64, unit: &str) -> String {
    let sep = if unit.is_empty() { "" } else { " " };
    if value == 0.0 || !value.is_finite() {
        return format!("{value}{sep}{unit}");
    }
    let exp = (value.abs().log10() / 3.0).floor() as i32 * 3;
    let exp = exp.clamp(-15, 12);
    let mut mantissa = value / 10f64.powi(exp);
    let mut exp = exp;
    // Rounding may reach 1000 (999.96 → 1000): move to the next prefix.
    if (round_sig(mantissa.abs(), 4) - 1000.0).abs() < 1e-9 && exp < 12 {
        mantissa /= 1000.0;
        exp += 3;
    }
    let p = match exp {
        -15 => "f",
        -12 => "p",
        -9 => "n",
        -6 => "µ",
        -3 => "m",
        0 => "",
        3 => "k",
        6 => "M",
        9 => "G",
        _ => "T",
    };
    let digits = 4 - (mantissa.abs().log10().floor() as i32 + 1);
    let digits = digits.clamp(0, 3) as usize;
    let mut text = format!("{:.*}", digits, round_sig(mantissa, 4));
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    let space = if unit.is_empty() && p.is_empty() { "" } else { " " };
    format!("{text}{space}{p}{unit}")
}

fn round_sig(v: f64, digits: i32) -> f64 {
    if v == 0.0 {
        return 0.0;
    }
    let scale = 10f64.powi(digits - 1 - v.abs().log10().floor() as i32);
    (v * scale).round() / scale
}

/// `value` for a SPICE netlist: 12 significant digits (so `100e-9` isn't
/// `1.0000000000000001e-7`), plain between 0.001 and 10⁶ (`4700`, `0.5`), else an exponent
/// (`1e7`, `4.7e-9`); never a SPICE suffix.
pub fn spice(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".into();
    }
    let e = format!("{value:.11e}");
    let (mantissa, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let (sign, mantissa) = mantissa.strip_prefix('-').map_or(("", mantissa), |m| ("-", m));
    let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    if (-3..6).contains(&exp) {
        // Plain: the point after `exp + 1` digits.
        let point = exp + 1;
        let text = if point <= 0 {
            format!("0.{}{digits}", "0".repeat((-point) as usize))
        } else if point as usize >= digits.len() {
            format!("{digits}{}", "0".repeat(point as usize - digits.len()))
        } else {
            format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
        };
        format!("{sign}{text}")
    } else {
        let m = if digits.len() > 1 { format!("{}.{}", &digits[..1], &digits[1..]) } else { digits.to_string() };
        format!("{sign}{m}e{exp}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() <= b.abs() * 1e-12)
    }

    #[test]
    fn parses_what_users_type() {
        for (text, value) in [
            ("10k", 10e3),
            ("4k7", 4.7e3),
            ("2R2", 2.2),
            ("5V1", 5.1),
            ("3n3", 3.3e-9),
            ("100n", 100e-9),
            ("10u", 10e-6),
            ("10µ", 10e-6),
            ("10μF", 10e-6),
            ("1M", 1e6),
            ("1m", 1e-3),
            ("1meg", 1e6),
            ("1Meg", 1e6),
            ("1e-3", 1e-3),
            ("2.5E3", 2.5e3),
            ("10 kΩ", 10e3),
            ("10kohm", 10e3),
            ("100nF 50V", 100e-9),
            ("10k 1%", 10e3),
            ("-12V", -12.0),
            ("+5", 5.0),
            ("0", 0.0),
            ("1.5ms", 1.5e-3),
            ("1kHz", 1e3),
            ("470", 470.0),
            ("0.1", 0.1),
            ("33R", 33.0),
            ("1 V", 1.0),
            ("2.2uH", 2.2e-6),
        ] {
            assert!(close(parse(text), value), "{text}: {:?}", parse(text));
        }
    }

    #[test]
    fn refuses_what_isnt_a_value() {
        for text in ["", "red", "BC847", "1N4148", "BZT52C3V3", "k", "10x", "2N7002", "LM358", "-", "."] {
            assert_eq!(parse(text), None, "{text}");
        }
    }

    #[test]
    fn formats_engineering_notation() {
        assert_eq!(format(4700.0, "Ω"), "4.7 kΩ");
        assert_eq!(format(0.0, "V"), "0 V");
        assert_eq!(format(-12.0, "V"), "-12 V");
        assert_eq!(format(1.5e-3, "s"), "1.5 ms");
        assert_eq!(format(1e-6, "s"), "1 µs");
        assert_eq!(format(999.96, "Hz"), "1 kHz");
        assert_eq!(format(123456.0, ""), "123.5 k");
        assert_eq!(format(1.0, ""), "1");
        assert_eq!(format(1e6, "Hz"), "1 MHz");
    }

    #[test]
    fn spice_numbers_have_no_suffix() {
        assert_eq!(spice(1e7), "1e7");
        assert_eq!(spice(4700.0), "4700");
        assert_eq!(spice(1e-9), "1e-9");
        assert_eq!(spice(0.0), "0");
        assert_eq!(spice(-0.5), "-0.5");
        assert_eq!(spice(100e-9), "1e-7");
        assert_eq!(spice(10e-6), "1e-5");
        assert_eq!(spice(0.001), "0.001");
        assert_eq!(spice(123456.0), "123456");
        assert_eq!(spice(-4.7e-9), "-4.7e-9");
        assert_eq!(spice(1.5e6), "1.5e6");
    }
}
