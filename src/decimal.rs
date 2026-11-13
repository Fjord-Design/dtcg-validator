use std::cmp::Ordering;

/// A borrowed view of a validated JSON number for comparisons with small bounds.
///
/// The significant digits stay in the original mantissa. Exponents are never
/// expanded into zero-filled buffers or converted through floating point.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Decimal<'a> {
    negative: bool,
    significant: &'a str,
    magnitude: i128,
}

impl<'a> Decimal<'a> {
    /// `lexeme` must already have been validated as a JSON number.
    pub(crate) fn from_json_number(lexeme: &'a str) -> Self {
        let negative = lexeme.starts_with('-');
        let unsigned = lexeme.strip_prefix('-').unwrap_or(lexeme);
        let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
        let Some(first_nonzero) = mantissa
            .bytes()
            .position(|byte| matches!(byte, b'1'..=b'9'))
        else {
            return Self {
                negative: false,
                significant: "",
                magnitude: 0,
            };
        };

        let integer_digits = mantissa.find('.').unwrap_or(mantissa.len());
        let leading_zeros = first_nonzero - usize::from(first_nonzero > integer_digits);
        let offset = integer_digits as i128 - leading_zeros as i128;

        Self {
            negative,
            significant: &mantissa[first_nonzero..],
            magnitude: parse_exponent(exponent).saturating_add(offset),
        }
    }

    /// Compares the exact value with an integer bound, including negative bounds.
    ///
    /// This deliberately does not implement decimal-to-decimal comparison:
    /// exponent saturation is exact for bounded comparisons, not for comparing
    /// two equally enormous exponents.
    pub(crate) fn cmp_integer(self, bound: i32) -> Ordering {
        if self.significant.is_empty() {
            return 0.cmp(&bound);
        }
        if bound == 0 {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        if self.negative != bound.is_negative() {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }

        let ordering = self.cmp_absolute_integer(bound.unsigned_abs());
        if self.negative {
            ordering.reverse()
        } else {
            ordering
        }
    }

    fn cmp_absolute_integer(self, mut bound: u32) -> Ordering {
        let mut divisor = 1;
        let mut bound_digits = 1;
        while bound / divisor >= 10 {
            divisor *= 10;
            bound_digits += 1;
        }

        let ordering = self.magnitude.cmp(&bound_digits);
        if ordering != Ordering::Equal {
            return ordering;
        }

        let mut digits = self.significant.bytes().filter(|byte| *byte != b'.');
        while divisor != 0 {
            let digit = digits.next().map_or(0, |byte| u32::from(byte - b'0'));
            let ordering = digit.cmp(&(bound / divisor));
            if ordering != Ordering::Equal {
                return ordering;
            }
            bound %= divisor;
            divisor /= 10;
        }

        if digits.any(|byte| byte != b'0') {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

fn parse_exponent(exponent: &str) -> i128 {
    let negative = exponent.starts_with('-');
    let digits = exponent.strip_prefix(['-', '+']).unwrap_or(exponent);
    let magnitude = digits.bytes().fold(0_i128, |value, byte| {
        value
            .saturating_mul(10)
            .saturating_add(i128::from(byte - b'0'))
    });

    // An exponent exceeding i128::MAX dominates every possible mantissa offset
    // (a usize-sized input length) and every i32 bound on native/WASM targets.
    // Saturation therefore cannot alter a bounded comparison, even when the
    // offset has the opposite sign. Negating the saturated positive magnitude
    // also avoids the i128::MIN negation overflow.
    if negative { -magnitude } else { magnitude }
}

#[cfg(test)]
mod tests {
    use super::Decimal;
    use std::cmp::Ordering::{Equal, Greater, Less};

    #[test]
    fn compares_exact_fractional_boundaries() {
        for (number, bound, expected) in [
            ("1000.0000000000000001", 1000, Greater),
            ("999.9999999999999999", 1000, Less),
            ("999.5", 1000, Less),
            ("1000.0000000000000000", 1000, Equal),
            ("0.99999999999999999999999999999", 1, Less),
            ("1.00000000000000000000000000001", 1, Greater),
            ("360", 360, Equal),
            ("359.99999999999999999999999999", 360, Less),
            ("-1000.0000000000000001", -1000, Less),
            ("-999.9999999999999999", -1000, Greater),
            ("-2147483648", i32::MIN, Equal),
            ("2147483647", i32::MAX, Equal),
        ] {
            assert_eq!(
                Decimal::from_json_number(number).cmp_integer(bound),
                expected,
                "{number} vs {bound}"
            );
        }
    }

    #[test]
    fn compares_shifted_mantissas_without_expansion() {
        for (number, bound, expected) in [
            ("1e1000", 1000, Greater),
            ("1e-1000", 1, Less),
            ("1e-1000", 0, Greater),
            ("-1e1000", -1000, Less),
            ("-1e-1000", 0, Less),
            ("0.001e3", 1, Equal),
            ("100000e-2", 1000, Equal),
            ("1.2300e+2", 123, Equal),
            ("0.000123E+6", 123, Equal),
            ("12300e-4", 1, Greater),
            ("12e-1", 2, Less),
            ("1.2e1", 12, Equal),
            ("1.20e1", 12, Equal),
            ("1.20001e1", 12, Greater),
        ] {
            assert_eq!(
                Decimal::from_json_number(number).cmp_integer(bound),
                expected,
                "{number} vs {bound}"
            );
        }
    }

    #[test]
    fn handles_exponents_beyond_machine_integer_ranges() {
        let huge = "999999999999999999999999999999999999999999999999999999999999999999999999999999";
        for (number, bound, expected) in [
            (format!("1e{huge}"), 1000, Greater),
            (format!("1e-{huge}"), 1, Less),
            (format!("1e-{huge}"), 0, Greater),
            (format!("-1e{huge}"), -1000, Less),
            (format!("-1e-{huge}"), 0, Less),
            (format!("0.00000000000000001e{huge}"), 1000, Greater),
            (format!("100000000000000000e-{huge}"), 1, Less),
            (format!("0e{huge}"), 0, Equal),
            (format!("-0.00e-{huge}"), 0, Equal),
        ] {
            assert_eq!(
                Decimal::from_json_number(&number).cmp_integer(bound),
                expected,
                "{number} vs {bound}"
            );
        }
    }

    #[test]
    fn treats_all_zero_lexemes_as_zero() {
        for number in ["0", "-0", "0.0", "-0.000", "0e+999", "-0E-999"] {
            let decimal = Decimal::from_json_number(number);
            assert_eq!(decimal.cmp_integer(0), Equal, "{number}");
            assert_eq!(decimal.cmp_integer(1), Less, "{number}");
            assert_eq!(decimal.cmp_integer(-1), Greater, "{number}");
        }
    }
}
