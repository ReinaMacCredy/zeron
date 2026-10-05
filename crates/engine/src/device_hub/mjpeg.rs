const BOUNDARY: &[u8] = b"--frame";
const HEADER_END: &[u8] = b"\r\n\r\n";
const MAX_BUFFER: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct MjpegParser {
    buf: Vec<u8>,
}

impl MjpegParser {
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(chunk);
        let mut frames = Vec::new();
        loop {
            let Some(start) = find(&self.buf, BOUNDARY) else {
                let keep = self.buf.len().min(BOUNDARY.len() - 1);
                self.buf.drain(..self.buf.len() - keep);
                break;
            };
            self.buf.drain(..start);
            let Some(header_len) = find(&self.buf, HEADER_END) else {
                if self.buf.len() > MAX_BUFFER {
                    self.buf.clear();
                }
                break;
            };
            let body_start = header_len + HEADER_END.len();
            let Some(length) = content_length(&self.buf[..header_len]) else {
                self.buf.drain(..body_start);
                continue;
            };
            if length > MAX_BUFFER {
                self.buf.drain(..body_start);
                continue;
            }
            if self.buf.len() < body_start + length {
                break;
            }
            frames.push(self.buf[body_start..body_start + length].to_vec());
            self.buf.drain(..body_start + length);
        }
        frames
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn content_length(headers: &[u8]) -> Option<usize> {
    std::str::from_utf8(headers).ok()?.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

#[cfg(test)]
pub(crate) fn part(body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out.extend_from_slice(b"\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_split_at_any_point_yield_exactly_their_bodies() {
        let bodies: Vec<Vec<u8>> = vec![
            b"\xff\xd8first\xff\xd9".to_vec(),
            b"--frame inside a body\r\n\r\n".to_vec(),
            (0..=255u8).collect(),
        ];
        let stream: Vec<u8> = bodies.iter().flat_map(|body| part(body)).collect();
        for chunk in 1..=stream.len() {
            let mut parser = MjpegParser::default();
            let frames: Vec<Vec<u8>> = stream
                .chunks(chunk)
                .flat_map(|piece| parser.push(piece))
                .collect();
            assert_eq!(frames, bodies, "chunk size {chunk}");
        }
    }

    #[test]
    fn a_short_final_chunk_yields_nothing() {
        let whole = part(b"complete");
        let mut parser = MjpegParser::default();
        assert_eq!(parser.push(&whole), vec![b"complete".to_vec()]);
        let next = part(b"truncated body");
        assert!(parser.push(&next[..next.len() - 6]).is_empty());
    }
}
