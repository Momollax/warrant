#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OptionKind {
    Call,
    Put,
}

#[derive(Debug, Clone, Copy)]
pub struct BlackScholesInput {
    pub kind: OptionKind,
    pub spot: f64,
    pub strike: f64,
    pub years_to_maturity: f64,
    pub risk_free_rate: f64,
    pub dividend_yield: f64,
    pub volatility: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct BlackScholesGreeks {
    pub delta: f64,
    pub gamma: f64,
    pub vega_per_vol_point: f64,
    pub theta_per_day: f64,
    pub rho_per_rate_point: f64,
    pub d1: f64,
    pub d2: f64,
}

pub fn black_scholes_price(input: BlackScholesInput) -> Option<f64> {
    validate_positive(input.spot)?;
    validate_positive(input.strike)?;
    validate_positive(input.years_to_maturity)?;
    validate_positive(input.volatility)?;

    let sqrt_t = input.years_to_maturity.sqrt();
    let d1 = ((input.spot / input.strike).ln()
        + (input.risk_free_rate - input.dividend_yield + 0.5 * input.volatility.powi(2))
            * input.years_to_maturity)
        / (input.volatility * sqrt_t);
    let d2 = d1 - input.volatility * sqrt_t;
    let discounted_spot = input.spot * (-input.dividend_yield * input.years_to_maturity).exp();
    let discounted_strike = input.strike * (-input.risk_free_rate * input.years_to_maturity).exp();

    let price = match input.kind {
        OptionKind::Call => discounted_spot * normal_cdf(d1) - discounted_strike * normal_cdf(d2),
        OptionKind::Put => discounted_strike * normal_cdf(-d2) - discounted_spot * normal_cdf(-d1),
    };

    price.is_finite().then_some(price.max(0.0))
}

pub fn black_scholes_greeks(input: BlackScholesInput) -> Option<BlackScholesGreeks> {
    validate_positive(input.spot)?;
    validate_positive(input.strike)?;
    validate_positive(input.years_to_maturity)?;
    validate_positive(input.volatility)?;

    let (d1, d2) = black_scholes_d1_d2(input)?;
    let sqrt_t = input.years_to_maturity.sqrt();
    let discounted_spot_factor = (-input.dividend_yield * input.years_to_maturity).exp();
    let discounted_strike_factor = (-input.risk_free_rate * input.years_to_maturity).exp();
    let pdf_d1 = normal_pdf(d1);
    let delta = match input.kind {
        OptionKind::Call => discounted_spot_factor * normal_cdf(d1),
        OptionKind::Put => -discounted_spot_factor * normal_cdf(-d1),
    };
    let gamma = discounted_spot_factor * pdf_d1 / (input.spot * input.volatility * sqrt_t);
    let vega_per_vol_point =
        input.spot * discounted_spot_factor * pdf_d1 * sqrt_t / 100.0;
    let theta_per_day = black_scholes_theta_per_year(input)? / 365.0;
    let rho_per_rate_point = match input.kind {
        OptionKind::Call => {
            input.strike
                * input.years_to_maturity
                * discounted_strike_factor
                * normal_cdf(d2)
                / 100.0
        }
        OptionKind::Put => {
            -input.strike
                * input.years_to_maturity
                * discounted_strike_factor
                * normal_cdf(-d2)
                / 100.0
        }
    };

    Some(BlackScholesGreeks {
        delta,
        gamma,
        vega_per_vol_point,
        theta_per_day,
        rho_per_rate_point,
        d1,
        d2,
    })
}

pub fn black_scholes_theta_per_year(input: BlackScholesInput) -> Option<f64> {
    validate_positive(input.spot)?;
    validate_positive(input.strike)?;
    validate_positive(input.years_to_maturity)?;
    validate_positive(input.volatility)?;

    let sqrt_t = input.years_to_maturity.sqrt();
    let d1 = ((input.spot / input.strike).ln()
        + (input.risk_free_rate - input.dividend_yield + 0.5 * input.volatility.powi(2))
            * input.years_to_maturity)
        / (input.volatility * sqrt_t);
    let d2 = d1 - input.volatility * sqrt_t;
    let discounted_spot = input.spot * (-input.dividend_yield * input.years_to_maturity).exp();
    let discounted_strike = input.strike * (-input.risk_free_rate * input.years_to_maturity).exp();
    let diffusion = -(discounted_spot * normal_pdf(d1) * input.volatility) / (2.0 * sqrt_t);

    let theta = match input.kind {
        OptionKind::Call => {
            diffusion - input.risk_free_rate * discounted_strike * normal_cdf(d2)
                + input.dividend_yield * discounted_spot * normal_cdf(d1)
        }
        OptionKind::Put => {
            diffusion + input.risk_free_rate * discounted_strike * normal_cdf(-d2)
                - input.dividend_yield * discounted_spot * normal_cdf(-d1)
        }
    };

    theta.is_finite().then_some(theta)
}

pub fn implied_volatility(
    kind: OptionKind,
    market_price: f64,
    spot: f64,
    strike: f64,
    years_to_maturity: f64,
    risk_free_rate: f64,
    dividend_yield: f64,
) -> Option<f64> {
    validate_positive(market_price)?;
    validate_positive(spot)?;
    validate_positive(strike)?;
    validate_positive(years_to_maturity)?;

    let intrinsic = discounted_intrinsic(
        kind,
        spot,
        strike,
        years_to_maturity,
        risk_free_rate,
        dividend_yield,
    );
    if market_price + 1e-9 < intrinsic {
        return None;
    }

    let mut low = 0.0001;
    let mut high = 5.0;
    let high_price = black_scholes_price(BlackScholesInput {
        kind,
        spot,
        strike,
        years_to_maturity,
        risk_free_rate,
        dividend_yield,
        volatility: high,
    })?;
    if high_price < market_price {
        return None;
    }

    for _ in 0..100 {
        let mid = (low + high) / 2.0;
        let price = black_scholes_price(BlackScholesInput {
            kind,
            spot,
            strike,
            years_to_maturity,
            risk_free_rate,
            dividend_yield,
            volatility: mid,
        })?;

        if (price - market_price).abs() <= 1e-6 {
            return Some(mid);
        }
        if price > market_price {
            high = mid;
        } else {
            low = mid;
        }
    }

    Some((low + high) / 2.0)
}

pub fn lognormal_probability(
    kind: OptionKind,
    spot: f64,
    level: f64,
    years: f64,
    drift: f64,
    volatility: f64,
) -> Option<(f64, f64)> {
    validate_positive(spot)?;
    validate_positive(level)?;
    validate_positive(years)?;
    validate_positive(volatility)?;

    let z = ((level / spot).ln() - (drift - 0.5 * volatility.powi(2)) * years)
        / (volatility * years.sqrt());
    let probability = match kind {
        OptionKind::Call => 1.0 - normal_cdf(z),
        OptionKind::Put => normal_cdf(z),
    };
    Some((probability.clamp(0.0, 1.0), z))
}

pub fn median_implied_volatility(values: impl IntoIterator<Item = Option<f64>>) -> Option<f64> {
    let mut vols = values
        .into_iter()
        .flatten()
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect::<Vec<_>>();
    vols.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));

    match vols.len() {
        0 => None,
        len if len % 2 == 1 => Some(vols[len / 2]),
        len => Some((vols[len / 2 - 1] + vols[len / 2]) / 2.0),
    }
}

fn discounted_intrinsic(
    kind: OptionKind,
    spot: f64,
    strike: f64,
    years_to_maturity: f64,
    risk_free_rate: f64,
    dividend_yield: f64,
) -> f64 {
    let discounted_spot = spot * (-dividend_yield * years_to_maturity).exp();
    let discounted_strike = strike * (-risk_free_rate * years_to_maturity).exp();
    match kind {
        OptionKind::Call => (discounted_spot - discounted_strike).max(0.0),
        OptionKind::Put => (discounted_strike - discounted_spot).max(0.0),
    }
}

fn black_scholes_d1_d2(input: BlackScholesInput) -> Option<(f64, f64)> {
    validate_positive(input.spot)?;
    validate_positive(input.strike)?;
    validate_positive(input.years_to_maturity)?;
    validate_positive(input.volatility)?;

    let sqrt_t = input.years_to_maturity.sqrt();
    let d1 = ((input.spot / input.strike).ln()
        + (input.risk_free_rate - input.dividend_yield + 0.5 * input.volatility.powi(2))
            * input.years_to_maturity)
        / (input.volatility * sqrt_t);
    let d2 = d1 - input.volatility * sqrt_t;
    Some((d1, d2))
}

pub fn normal_cdf(x: f64) -> f64 {
    // Abramowitz-Stegun approximation, sufficient for pricing diagnostics.
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let z = x.abs() / 2.0_f64.sqrt();
    0.5 * (1.0 + sign * erf(z))
}

pub fn normal_pdf(x: f64) -> f64 {
    (-0.5 * x * x).exp() / (2.0 * std::f64::consts::PI).sqrt()
}

fn erf(x: f64) -> f64 {
    let a1 = 0.254829592;
    let a2 = -0.284496736;
    let a3 = 1.421413741;
    let a4 = -1.453152027;
    let a5 = 1.061405429;
    let p = 0.3275911;

    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + p * x);
    let y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * (-x * x).exp();
    sign * y
}

fn validate_positive(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_scholes_matches_reference_call_price() {
        let price = black_scholes_price(BlackScholesInput {
            kind: OptionKind::Call,
            spot: 100.0,
            strike: 100.0,
            years_to_maturity: 1.0,
            risk_free_rate: 0.05,
            dividend_yield: 0.0,
            volatility: 0.20,
        })
        .unwrap();

        assert_close(price, 10.4506, 1e-3);
    }

    #[test]
    fn implied_volatility_recovers_reference_volatility() {
        let price = black_scholes_price(BlackScholesInput {
            kind: OptionKind::Put,
            spot: 100.0,
            strike: 105.0,
            years_to_maturity: 0.5,
            risk_free_rate: 0.03,
            dividend_yield: 0.01,
            volatility: 0.35,
        })
        .unwrap();

        let iv = implied_volatility(OptionKind::Put, price, 100.0, 105.0, 0.5, 0.03, 0.01)
            .unwrap();

        assert_close(iv, 0.35, 1e-4);
    }

    #[test]
    fn black_scholes_theta_is_negative_for_atm_call() {
        let theta = black_scholes_theta_per_year(BlackScholesInput {
            kind: OptionKind::Call,
            spot: 100.0,
            strike: 100.0,
            years_to_maturity: 1.0,
            risk_free_rate: 0.05,
            dividend_yield: 0.0,
            volatility: 0.20,
        })
        .unwrap();

        assert!(theta < 0.0, "theta={theta}");
    }

    #[test]
    fn black_scholes_greeks_match_reference_signs_and_scales() {
        let greeks = black_scholes_greeks(BlackScholesInput {
            kind: OptionKind::Call,
            spot: 100.0,
            strike: 100.0,
            years_to_maturity: 1.0,
            risk_free_rate: 0.05,
            dividend_yield: 0.0,
            volatility: 0.20,
        })
        .unwrap();

        assert_close(greeks.delta, 0.6368, 1e-3);
        assert_close(greeks.gamma, 0.0188, 1e-3);
        assert_close(greeks.vega_per_vol_point, 0.3752, 1e-3);
        assert!(greeks.theta_per_day < 0.0);
        assert!(greeks.rho_per_rate_point > 0.0);
        assert!(greeks.d1 > greeks.d2);
    }

    #[test]
    fn lognormal_probability_direction_depends_on_option_side() {
        let call = lognormal_probability(OptionKind::Call, 100.0, 110.0, 1.0, 0.0, 0.20)
            .unwrap();
        let put = lognormal_probability(OptionKind::Put, 100.0, 90.0, 1.0, 0.0, 0.20)
            .unwrap();

        assert!(call.0 > 0.20 && call.0 < 0.40, "call={call:?}");
        assert!(put.0 > 0.20 && put.0 < 0.40, "put={put:?}");
        assert!(call.1.is_finite());
        assert!(put.1.is_finite());
    }

    #[test]
    fn near_term_warrant_realistic_data_produces_finite_iv() {
        let spot = 300.23;
        let strike = 260.0;
        let eur_price = 3.65;
        let usd_to_eur = 0.8603;
        let parity = 10.0;
        let market_price_per_underlying_usd = eur_price / usd_to_eur * parity;
        let years = 32.0 / 365.0;

        let iv = implied_volatility(
            OptionKind::Call,
            market_price_per_underlying_usd,
            spot,
            strike,
            years,
            0.045,
            0.005,
        )
        .unwrap();

        assert!(iv > 0.10 && iv < 2.00, "iv={iv}");
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual={actual}, expected={expected}, tolerance={tolerance}"
        );
    }
}
