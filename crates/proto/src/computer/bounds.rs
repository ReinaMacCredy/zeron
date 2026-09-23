use serde::de::{Deserialize, Deserializer, Error};

pub(crate) const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

// Effect's length checks count UTF-16 code units, like a JS string's length.
pub(crate) fn js_len(value: &str) -> usize {
    value.encode_utf16().count()
}

pub(crate) fn js_slice(value: &str, max: usize) -> String {
    let mut units = 0;
    value
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= max
        })
        .collect()
}

pub(crate) fn safe_integer(value: &serde_json::Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
        return (n.abs() <= MAX_SAFE_INTEGER).then_some(n);
    }
    let n = value.as_f64()?;
    (n.fract() == 0.0 && n.abs() <= MAX_SAFE_INTEGER as f64).then_some(n as i64)
}

fn check_text(value: String, max: usize) -> Result<String, String> {
    if js_len(&value) > max {
        return Err(format!("expected at most {max} characters"));
    }
    Ok(value)
}

fn check_trimmed(value: String, max: usize) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("expected a non-empty string".into());
    }
    check_text(trimmed.to_string(), max)
}

fn check_int<T: TryFrom<i64>>(value: i64, min: i64, max: i64) -> Result<T, String> {
    if value < min || value > max {
        return Err(format!("expected an integer between {min} and {max}"));
    }
    T::try_from(value).map_err(|_| format!("integer {value} out of range"))
}

fn check_items<T>(items: Vec<T>, min: usize, max: usize) -> Result<Vec<T>, String> {
    if items.len() < min || items.len() > max {
        return Err(format!("expected between {min} and {max} items"));
    }
    Ok(items)
}

fn check_matching(value: String, valid: fn(&str) -> bool) -> Result<String, String> {
    if valid(&value) {
        Ok(value)
    } else {
        Err(format!("{value:?} does not match the expected pattern"))
    }
}

pub(crate) fn text<'de, D: Deserializer<'de>, const MAX: usize>(d: D) -> Result<String, D::Error> {
    check_text(String::deserialize(d)?, MAX).map_err(D::Error::custom)
}

pub(crate) fn opt_text<'de, D: Deserializer<'de>, const MAX: usize>(
    d: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)?
        .map(|v| check_text(v, MAX))
        .transpose()
        .map_err(D::Error::custom)
}

pub(crate) fn trimmed<'de, D: Deserializer<'de>, const MAX: usize>(
    d: D,
) -> Result<String, D::Error> {
    check_trimmed(String::deserialize(d)?, MAX).map_err(D::Error::custom)
}

pub(crate) fn opt_trimmed<'de, D: Deserializer<'de>, const MAX: usize>(
    d: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)?
        .map(|v| check_trimmed(v, MAX))
        .transpose()
        .map_err(D::Error::custom)
}

pub(crate) fn int<'de, D, T, const MIN: i64, const MAX: i64>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: TryFrom<i64>,
{
    check_int(i64::deserialize(d)?, MIN, MAX).map_err(D::Error::custom)
}

pub(crate) fn opt_int<'de, D, T, const MIN: i64, const MAX: i64>(
    d: D,
) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: TryFrom<i64>,
{
    Option::<i64>::deserialize(d)?
        .map(|v| check_int(v, MIN, MAX))
        .transpose()
        .map_err(D::Error::custom)
}

pub(crate) fn positive<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    match Option::<f64>::deserialize(d)? {
        Some(v) if v <= 0.0 => Err(D::Error::custom("expected a positive number")),
        other => Ok(other),
    }
}

pub(crate) fn non_negative<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    if v < 0.0 {
        return Err(D::Error::custom("expected a non-negative number"));
    }
    Ok(v)
}

pub(crate) fn within<'de, D: Deserializer<'de>, const LIMIT: i64>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    if v.abs() > LIMIT as f64 {
        return Err(D::Error::custom(format!(
            "expected a value within ±{LIMIT}"
        )));
    }
    Ok(v)
}

pub(crate) fn items<'de, D, T, const MIN: usize, const MAX: usize>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    check_items(Vec::<T>::deserialize(d)?, MIN, MAX).map_err(D::Error::custom)
}

pub(crate) fn opt_items<'de, D, T, const MIN: usize, const MAX: usize>(
    d: D,
) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<Vec<T>>::deserialize(d)?
        .map(|v| check_items(v, MIN, MAX))
        .transpose()
        .map_err(D::Error::custom)
}

fn check_trimmed_items(
    items: Vec<String>,
    min: usize,
    max: usize,
    len: usize,
) -> Result<Vec<String>, String> {
    check_items(items, min, max)?
        .into_iter()
        .map(|v| check_trimmed(v, len))
        .collect()
}

pub(crate) fn trimmed_items<'de, D, const MIN: usize, const MAX: usize, const LEN: usize>(
    d: D,
) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    check_trimmed_items(Vec::<String>::deserialize(d)?, MIN, MAX, LEN).map_err(D::Error::custom)
}

pub(crate) fn opt_trimmed_items<'de, D, const MAX: usize, const LEN: usize>(
    d: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Vec<String>>::deserialize(d)?
        .map(|items| check_trimmed_items(items, 0, MAX, LEN))
        .transpose()
        .map_err(D::Error::custom)
}

pub(crate) fn opt_text_items<'de, D, const MAX: usize, const LEN: usize>(
    d: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Vec<String>>::deserialize(d)?
        .map(|items| {
            check_items(items, 0, MAX)
                .and_then(|items| items.into_iter().map(|v| check_text(v, LEN)).collect())
        })
        .transpose()
        .map_err(D::Error::custom)
}

pub(crate) fn matching<'de, D: Deserializer<'de>>(
    d: D,
    valid: fn(&str) -> bool,
) -> Result<String, D::Error> {
    check_matching(String::deserialize(d)?, valid).map_err(D::Error::custom)
}

pub(crate) fn opt_matching<'de, D: Deserializer<'de>>(
    d: D,
    valid: fn(&str) -> bool,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)?
        .map(|v| check_matching(v, valid))
        .transpose()
        .map_err(D::Error::custom)
}

pub(crate) fn chars_in(value: &str, min: usize, max: usize, allowed: fn(char) -> bool) -> bool {
    let len = value.chars().count();
    len >= min && len <= max && value.chars().all(allowed)
}
