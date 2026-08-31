//! A narrow HTTP/1.1 client — only what the Docker Engine API needs.
//!
//! This is a Docker transport, not an HTTP library. It implements request
//! lines, a fixed header set, JSON request bodies, and `Content-Length` or
//! chunked response bodies. It deliberately does not do redirects, cookies,
//! compression, proxies, authentication, retries or connection reuse: Docker
//! calls are infrequent and streams are long-lived, so pooling would add state
//! to save nothing.
//!
//! One request per connection, using `Connection: close`.

use std::io::{BufRead, BufReader, Read, Write};

use super::transport::Stream;
use super::DockerError;

/// A complete (non-streaming) HTTP response.
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Perform one request on a freshly connected stream.
pub fn request(
    stream: Box<dyn Stream>,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> Result<Response, DockerError> {
    let mut stream = stream;
    write_request(&mut stream, method, path, body).map_err(io_error)?;

    let mut reader = BufReader::new(stream);
    let status = read_status_line(&mut reader)?;
    let headers = read_headers(&mut reader)?;
    let body = read_body(&mut reader, &headers)?;

    Ok(Response { status, body })
}

fn write_request(
    stream: &mut Box<dyn Stream>,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> std::io::Result<()> {
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: docker\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n"
    );
    if let Some(body) = body {
        head.push_str("Content-Type: application/json\r\n");
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");

    stream.write_all(head.as_bytes())?;
    if let Some(body) = body {
        stream.write_all(body)?;
    }
    stream.flush()
}

/// Parse `HTTP/1.1 200 OK` into its status code.
fn read_status_line<R: BufRead>(reader: &mut R) -> Result<u16, DockerError> {
    let line = read_line(reader)?;
    let mut parts = line.split_whitespace();

    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/") {
        return Err(DockerError::Protocol(format!(
            "expected an HTTP status line, got {line:?}"
        )));
    }

    parts
        .next()
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| DockerError::Protocol(format!("no status code in {line:?}")))
}

/// Read headers up to the blank line, lowercasing names for lookup.
fn read_headers<R: BufRead>(reader: &mut R) -> Result<Vec<(String, String)>, DockerError> {
    let mut headers = Vec::new();
    loop {
        let line = read_line(reader)?;
        if line.is_empty() {
            return Ok(headers);
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

fn read_body<R: BufRead>(
    reader: &mut R,
    headers: &[(String, String)],
) -> Result<Vec<u8>, DockerError> {
    let chunked = header(headers, "transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));

    if chunked {
        return read_chunked(reader);
    }

    match header(headers, "content-length").and_then(|v| v.parse::<usize>().ok()) {
        Some(len) => {
            let mut body = vec![0; len];
            reader.read_exact(&mut body).map_err(io_error)?;
            Ok(body)
        }
        // No length and no chunking: `Connection: close` means read to EOF.
        None => {
            let mut body = Vec::new();
            reader.read_to_end(&mut body).map_err(io_error)?;
            Ok(body)
        }
    }
}

/// Decode a chunked body: size line, that many bytes, CRLF, until a zero chunk.
fn read_chunked<R: BufRead>(reader: &mut R) -> Result<Vec<u8>, DockerError> {
    let mut body = Vec::new();
    loop {
        let size = read_chunk_size(reader)?;
        if size == 0 {
            // Trailing headers, then the final blank line.
            while !read_line(reader)?.is_empty() {}
            return Ok(body);
        }

        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..]).map_err(io_error)?;

        // Each chunk's data is followed by CRLF.
        read_line(reader)?;
    }
}

/// A chunk size is hex, optionally followed by `;extension`.
fn read_chunk_size<R: BufRead>(reader: &mut R) -> Result<usize, DockerError> {
    let line = read_line(reader)?;
    let digits = line.split(';').next().unwrap_or_default().trim();
    usize::from_str_radix(digits, 16)
        .map_err(|_| DockerError::Protocol(format!("invalid chunk size {line:?}")))
}

/// Read one CRLF-terminated line, without the terminator.
fn read_line<R: BufRead>(reader: &mut R) -> Result<String, DockerError> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).map_err(io_error)?;
    if read == 0 {
        return Err(DockerError::Protocol(
            "connection closed mid-response".to_string(),
        ));
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

fn io_error(err: std::io::Error) -> DockerError {
    use std::io::ErrorKind::*;

    match err.kind() {
        TimedOut | WouldBlock => DockerError::Timeout,
        UnexpectedEof => DockerError::Protocol("connection closed mid-response".to_string()),
        _ => DockerError::Protocol(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn reader(raw: &str) -> BufReader<Cursor<Vec<u8>>> {
        BufReader::new(Cursor::new(raw.as_bytes().to_vec()))
    }

    #[test]
    fn reads_status_code() {
        assert_eq!(
            read_status_line(&mut reader("HTTP/1.1 200 OK\r\n")).unwrap(),
            200
        );
    }

    #[test]
    fn reads_status_code_without_reason() {
        assert_eq!(
            read_status_line(&mut reader("HTTP/1.1 204\r\n")).unwrap(),
            204
        );
    }

    #[test]
    fn rejects_a_non_http_status_line() {
        assert!(read_status_line(&mut reader("garbage\r\n")).is_err());
    }

    #[test]
    fn reads_headers_case_insensitively() {
        let mut r = reader("Content-Type: application/json\r\nAPI-Version: 1.52\r\n\r\n");
        let headers = read_headers(&mut r).unwrap();
        assert_eq!(header(&headers, "content-type"), Some("application/json"));
        assert_eq!(header(&headers, "api-version"), Some("1.52"));
    }

    #[test]
    fn reads_a_content_length_body() {
        let mut r = reader("hello world");
        let headers = vec![("content-length".to_string(), "5".to_string())];
        assert_eq!(read_body(&mut r, &headers).unwrap(), b"hello");
    }

    #[test]
    fn reads_a_body_without_length_to_eof() {
        let mut r = reader("OK");
        assert_eq!(read_body(&mut r, &[]).unwrap(), b"OK");
    }

    #[test]
    fn decodes_a_chunked_body() {
        let mut r = reader("5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n");
        assert_eq!(read_chunked(&mut r).unwrap(), b"hello world");
    }

    #[test]
    fn decodes_a_chunked_body_with_extensions() {
        let mut r = reader("5;first=1\r\nhello\r\n0\r\n\r\n");
        assert_eq!(read_chunked(&mut r).unwrap(), b"hello");
    }

    #[test]
    fn decodes_an_empty_chunked_body() {
        let mut r = reader("0\r\n\r\n");
        assert!(read_chunked(&mut r).unwrap().is_empty());
    }

    #[test]
    fn decodes_a_chunked_body_with_trailers() {
        let mut r = reader("2\r\nhi\r\n0\r\nX-Trailer: v\r\n\r\n");
        assert_eq!(read_chunked(&mut r).unwrap(), b"hi");
    }

    #[test]
    fn decodes_chunked_binary_data() {
        // A log frame header is binary and contains a zero byte; it must survive.
        let mut raw = b"8\r\n".to_vec();
        raw.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 5]);
        raw.extend_from_slice(b"\r\n0\r\n\r\n");

        let mut r = BufReader::new(Cursor::new(raw));
        assert_eq!(read_chunked(&mut r).unwrap(), vec![1, 0, 0, 0, 0, 0, 0, 5]);
    }

    #[test]
    fn rejects_an_invalid_chunk_size() {
        let mut r = reader("zz\r\n");
        assert!(read_chunked(&mut r).is_err());
    }

    #[test]
    fn reports_a_truncated_response() {
        let mut r = reader("");
        assert!(read_line(&mut r).is_err());
    }
}
