pub const SCROLL_STEP_PX: f64 = 80.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiscreteSteps {
    pub steps: i64,
    pub remainder: f64,
}

pub fn take_discrete_steps(remainder: f64, delta: f64) -> DiscreteSteps {
    if !delta.is_finite() || delta == 0.0 {
        return DiscreteSteps {
            steps: 0,
            remainder,
        };
    }
    let carried = remainder + delta;
    let steps = (carried / SCROLL_STEP_PX).trunc() as i64;
    DiscreteSteps {
        steps,
        remainder: carried - steps as f64 * SCROLL_STEP_PX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(steps: i64, remainder: f64) -> DiscreteSteps {
        DiscreteSteps { steps, remainder }
    }

    #[test]
    fn converts_exact_multiples_of_a_notch_exactly() {
        assert_eq!(take_discrete_steps(0.0, -600.0), steps(-7, -40.0));
    }

    #[test]
    fn truncates_a_sub_notch_delta_to_zero_steps_and_carries_the_dust() {
        let first = take_discrete_steps(0.0, 50.0);
        assert_eq!(first, steps(0, 50.0));
        assert_eq!(take_discrete_steps(first.remainder, 50.0), steps(1, 20.0));
    }

    #[test]
    fn truncates_toward_zero_in_both_directions() {
        assert_eq!(take_discrete_steps(0.0, 79.0), steps(0, 79.0));
        assert_eq!(take_discrete_steps(0.0, -79.0), steps(0, -79.0));
    }

    #[test]
    fn carries_negative_remainders_the_same_way() {
        let mut total = 0;
        let mut remainder = 0.0;
        for _ in 0..6 {
            let step = take_discrete_steps(remainder, -20.0);
            total += step.steps;
            remainder = step.remainder;
        }
        assert_eq!(total, -1);
        assert_eq!(remainder, -40.0);
    }

    #[test]
    fn ignores_non_finite_deltas_instead_of_poisoning_the_accumulator() {
        assert_eq!(take_discrete_steps(3.0, f64::NAN), steps(0, 3.0));
        assert_eq!(take_discrete_steps(3.0, f64::INFINITY), steps(0, 3.0));
    }

    #[test]
    fn keeps_the_notch_size_the_native_implementations_document() {
        assert_eq!(SCROLL_STEP_PX, 80.0);
    }
}
