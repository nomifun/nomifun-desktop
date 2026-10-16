//! Shared numerical and input/result-correlation checks for retrieval codecs.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::error::{InvokeError, InvokeErrorKind};

pub(crate) fn finite_f32(value: &Value, context: &str) -> Result<f32, InvokeError> {
    let number = value.as_f64().filter(|number| number.is_finite())
        .ok_or_else(|| InvokeError::parse(format!("{context} must be a finite number")))?;
    let converted = number as f32;
    if !converted.is_finite() {
        return Err(InvokeError::parse(format!("{context} exceeds the finite f32 range")));
    }
    Ok(converted)
}

pub(crate) fn result_index(value: &Value, input_count: usize, seen: &mut BTreeSet<usize>, context: &str) -> Result<usize, InvokeError> {
    let index = value.as_u64().and_then(|index| usize::try_from(index).ok())
        .filter(|index| *index < input_count)
        .ok_or_else(|| InvokeError::parse(format!("{context} has an invalid or out-of-range index")))?;
    if !seen.insert(index) {
        return Err(InvokeError::parse(format!("{context} contains a duplicate input index")));
    }
    Ok(index)
}

pub(crate) fn requested_dimension(value: Option<&Value>, context: &str) -> Result<Option<usize>, InvokeError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().and_then(|dimension| usize::try_from(dimension).ok())
            .filter(|dimension| *dimension > 0).map(Some)
            .ok_or_else(|| InvokeError::new(InvokeErrorKind::InvalidParams, format!("{context} must be a positive integer"))),
    }
}

/// Entirely unindexed compatible responses retain array order. Once any row
/// has an index, every row must uniquely cover one of the requested inputs.
pub(crate) fn embedding_batch(
    rows: &[Value], expected_count: usize, index_field: &str, dimension: Option<usize>, context: &str,
) -> Result<Vec<Vec<f32>>, InvokeError> {
    if rows.is_empty() || rows.len() != expected_count {
        return Err(InvokeError::parse(format!("{context} count must match the non-empty input batch")));
    }
    let indexed = rows.iter().any(|row| row.get(index_field).is_some());
    let mut seen = BTreeSet::new();
    let mut expected_dimension = dimension;
    let mut vectors = Vec::with_capacity(rows.len());
    for (position, row) in rows.iter().enumerate() {
        let index = if indexed {
            result_index(row.get(index_field).ok_or_else(|| InvokeError::parse(format!("{context} contains partial indices")))?,
                expected_count, &mut seen, context)?
        } else { position };
        let values = row.get("embedding").and_then(Value::as_array).filter(|values| !values.is_empty())
            .ok_or_else(|| InvokeError::parse(format!("{context} vector must be a non-empty array")))?;
        if values.len() != *expected_dimension.get_or_insert(values.len()) {
            return Err(InvokeError::parse(format!("{context} vector dimensions do not match")));
        }
        let vector = values.iter().map(|value| finite_f32(value, context)).collect::<Result<Vec<_>, _>>()?;
        vectors.push((index, vector));
    }
    vectors.sort_by_key(|(index, _)| *index);
    Ok(vectors.into_iter().map(|(_, vector)| vector).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finite_values_preserve_zero_and_negative_values_without_an_invented_score_range() {
        for number in [0.0, -0.5, -12.0, 2.5, f32::MAX as f64, f32::MIN as f64] {
            assert_eq!(finite_f32(&json!(number), "result").unwrap(), number as f32);
        }
        for value in [json!(1e100), json!(-1e100), json!(null), json!("NaN"), json!("0.5"), json!({})] {
            assert_eq!(finite_f32(&value, "result").unwrap_err().kind, InvokeErrorKind::ParseError);
        }
    }

    #[test]
    fn requested_dimensions_are_optional_positive_integers_without_coercion() {
        assert_eq!(requested_dimension(None, "dimension").unwrap(), None);
        assert_eq!(requested_dimension(Some(&Value::Null), "dimension").unwrap(), None);
        assert_eq!(requested_dimension(Some(&json!(2)), "dimension").unwrap(), Some(2));
        for value in [json!(0), json!(-1), json!(1.5), json!("2"), json!({})] {
            assert_eq!(requested_dimension(Some(&value), "dimension").unwrap_err().kind, InvokeErrorKind::InvalidParams);
        }
    }

    #[test]
    fn embedding_batches_reject_empty_inconsistent_non_numeric_and_overflowing_vectors() {
        for (rows, expected, dimension) in [
            (json!([]), 1, None),
            (json!([{"embedding":[]}]), 1, None),
            (json!([{"embedding":[1]}]), 2, None),
            (json!([{"embedding":[1]},{"embedding":[1,2]}]), 2, None),
            (json!([{"embedding":[1]}]), 1, Some(2)),
            (json!([{"embedding":[1e100]}]), 1, None),
            (json!([{"embedding":[null]}]), 1, None),
        ] {
            assert_eq!(embedding_batch(rows.as_array().unwrap(), expected, "index", dimension, "fixture").unwrap_err().kind,
                InvokeErrorKind::ParseError);
        }
    }

    #[test]
    fn indexed_batches_require_complete_unique_integer_input_indices() {
        for index_field in ["index", "text_index"] {
            for indices in [vec![json!(0), json!(0)], vec![json!(0), json!(2)], vec![json!(-1), json!(1)],
                vec![json!(0.5), json!(1)], vec![json!("0"), json!(1)], vec![json!(null), json!(1)]] {
                let rows: Vec<_> = indices.into_iter().map(|index| json!({index_field:index,"embedding":[1.0]})).collect();
                assert_eq!(embedding_batch(&rows, 2, index_field, None, "fixture").unwrap_err().kind, InvokeErrorKind::ParseError);
            }
            let mixed = vec![json!({index_field:0,"embedding":[1]}), json!({"embedding":[2]})];
            assert_eq!(embedding_batch(&mixed, 2, index_field, None, "fixture").unwrap_err().kind, InvokeErrorKind::ParseError);
            let shuffled = vec![json!({index_field:1,"embedding":[0,-2]}), json!({index_field:0,"embedding":[3,4]})];
            assert_eq!(embedding_batch(&shuffled, 2, index_field, Some(2), "fixture").unwrap(), vec![vec![3.0,4.0],vec![0.0,-2.0]]);
        }
    }
}
