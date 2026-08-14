/// Read an HTTP response incrementally and stop before it can exceed the
/// caller's memory limit. `Content-Length` is only an early rejection hint;
/// chunked or dishonest responses are still bounded while streaming.
pub async fn read_limited(
    mut response: reqwest::Response,
    max_bytes: usize,
    too_large: String,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(too_large);
    }

    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(0)
        .min(max_bytes);
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("response body: {error}"))?
    {
        append_limited(&mut body, &chunk, max_bytes, &too_large)?;
    }
    Ok(body)
}

fn append_limited(
    body: &mut Vec<u8>,
    chunk: &[u8],
    max_bytes: usize,
    too_large: &str,
) -> Result<(), String> {
    if chunk.len() > max_bytes.saturating_sub(body.len()) {
        return Err(too_large.to_string());
    }
    body.extend_from_slice(chunk);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_chunk_before_growing_past_limit() {
        let mut body = vec![1, 2, 3];
        assert!(append_limited(&mut body, &[4, 5], 4, "too large").is_err());
        assert_eq!(body, vec![1, 2, 3]);
    }

    #[test]
    fn accepts_body_exactly_at_limit() {
        let mut body = vec![1, 2];
        append_limited(&mut body, &[3, 4], 4, "too large").expect("append at limit");
        assert_eq!(body, vec![1, 2, 3, 4]);
    }
}
