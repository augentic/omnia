//! `Cache-Control` and `If-None-Match` request header parsing.

use anyhow::{Result, bail};
use http::{HeaderMap, HeaderValue};
use http::header::{CACHE_CONTROL, IF_NONE_MATCH};

use crate::Error;

/// Caching directives parsed from a request's `Cache-Control` and
/// `If-None-Match` headers.
#[derive(Clone, Debug, Default)]
pub struct Control {
    no_cache: bool,
    no_store: bool,
    // zero when the directive is absent, and zero neither serves nor stores
    max_age: u64,
    // empty under `no-store`, the one directive that needs no entity-tag
    etag: String,
}

impl Control {
    /// The caching directives in `headers`, or `None` when their
    /// `Cache-Control` holds none the [cache](crate::http::cache) acts on.
    ///
    /// # Errors
    ///
    /// [`ServerError`](crate::ErrorKind::ServerError) with the code `invalid` when the
    /// caching headers are malformed or conflict.
    pub fn maybe_from(headers: &HeaderMap) -> crate::Result<Option<Self>> {
        if headers.get(CACHE_CONTROL).is_none() {
            tracing::debug!("no Cache-Control header present");
            return Ok(None);
        }
        Self::parse(headers).map_err(|e| {
            Error::server_error("invalid", format!("issue parsing Cache-Control headers: {e:#}"))
        })
    }

    /// Whether a stored response may answer the request.
    #[must_use]
    pub const fn reads(&self) -> bool {
        !self.no_cache && !self.no_store && self.max_age > 0 && !self.etag.is_empty()
    }

    /// Whether a successful origin response is stored.
    #[must_use]
    pub const fn writes(&self) -> bool {
        !self.no_store && self.max_age > 0 && !self.etag.is_empty()
    }

    /// The strong entity-tag from `If-None-Match`, empty under `no-store`.
    ///
    /// The cache stores its entry at [`store_key`](super::store_key) of the
    /// tag.
    #[must_use]
    pub fn etag(&self) -> &str {
        &self.etag
    }

    /// The `max-age` directive in seconds, zero when absent.
    #[must_use]
    pub const fn max_age(&self) -> u64 {
        self.max_age
    }
}

impl Control {
    fn parse(headers: &HeaderMap) -> Result<Option<Self>> {
        // read the directives of every line as one set
        let mut control = Self::default();
        let mut max_age = None;
        for line in headers.get_all(CACHE_CONTROL) {
            for directive in field_value(line)?.split(',') {
                let directive = directive.trim().to_ascii_lowercase();
                if directive == "no-store" {
                    control.no_store = true;
                } else if directive == "no-cache" {
                    control.no_cache = true;
                } else if let Some(value) = directive.strip_prefix("max-age=") {
                    if max_age.is_some() {
                        bail!("`max-age` directive given more than once");
                    }
                    let Ok(secs) = value.trim().parse() else {
                        bail!("`max-age` directive is malformed");
                    };
                    max_age = Some(secs);
                }
            }
        }

        // pass through a request with no directive the cache acts on
        if !control.no_store && !control.no_cache && max_age.is_none() {
            tracing::debug!("no recognised Cache-Control directive present");
            return Ok(None);
        }

        // refuse conflicting directives
        if control.no_store && (control.no_cache || max_age.is_some()) {
            bail!("`no-store` cannot be combined with `no-cache` or `max-age`");
        }
        control.max_age = max_age.unwrap_or(0);

        // take the one strong entity-tag that names the entry
        if !control.no_store {
            let mut lines = headers.get_all(IF_NONE_MATCH).iter();
            let Some(etag) = lines.next() else {
                bail!(
                    "`If-None-Match` header required when using `Cache-Control: max-age` or `no-cache`"
                );
            };
            if lines.next().is_some() {
                bail!("multiple `etag` values in `If-None-Match` header are not supported");
            }
            control.etag = strong_etag(field_value(etag)?)?.to_string();
        }

        Ok(Some(control))
    }
}

// RFC 9110 §5.5 field values may pad with OWS (SP / HTAB). `HeaderValue::to_str`
// rejects HTAB, so [`field_value`] reads bytes and refuses only `obs-text`.
fn field_value(value: &HeaderValue) -> Result<&str> {
    let bytes = value.as_bytes();
    for &octet in bytes {
        if octet == b'\t' || (0x20..=0x7E).contains(&octet) {
            continue;
        }
        if octet >= 0x80 {
            bail!("header value holds `obs-text` octets, which this cache does not accept");
        }
        bail!("header value holds invalid octets");
    }
    Ok(std::str::from_utf8(bytes)?)
}

// RFC 9110 §8.8.3 has `opaque-tag = DQUOTE *etagc DQUOTE`, and `etagc`
// admits a comma but no DQUOTE. So the first quote after the opening one
// closes the tag, and anything past it makes a list. The caller has refused
// `obs-text`, which leaves `etagc` ASCII.
fn strong_etag(value: &str) -> Result<&str> {
    let value = value.trim_matches([' ', '\t']);
    if value.is_empty() {
        bail!("`If-None-Match` header is empty");
    }
    if value == "*" {
        bail!("`If-None-Match: *` is not a usable cache key");
    }
    if value.starts_with("W/") {
        bail!("weak `etag` values in `If-None-Match` header are not supported");
    }
    let Some(body) = value.strip_prefix('"') else {
        bail!("`If-None-Match` value is not a quoted entity-tag");
    };
    let Some(close) = body.find('"') else {
        bail!("`If-None-Match` entity-tag is missing its closing quote");
    };
    let (opaque, rest) = body.split_at(close);
    if !rest[1..].trim_matches([' ', '\t']).is_empty() {
        bail!("multiple `etag` values in `If-None-Match` header are not supported");
    }
    if !opaque.bytes().all(|b| b == 0x21 || (0x23..=0x7E).contains(&b)) {
        bail!("`If-None-Match` entity-tag contains characters outside `etagc`");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ows_tab_in_cache_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_bytes(b"\tmax-age=60").expect("tab is valid OWS"),
        );
        headers.insert(
            IF_NONE_MATCH,
            HeaderValue::from_bytes(b"\t\"v1\"\t").expect("tab is valid OWS"),
        );
        let control = Control::maybe_from(&headers)
            .expect("parses")
            .expect("recognised directive");
        assert_eq!(control.max_age(), 60);
        assert_eq!(control.etag(), "\"v1\"");
    }

    #[test]
    fn if_none_match_obs_text_not_ows() {
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("max-age=60"));
        headers.insert(
            IF_NONE_MATCH,
            HeaderValue::from_bytes(b"\xE9").expect("obs-text is a legal field value"),
        );
        let err = Control::maybe_from(&headers).expect_err("refuses obs-text");
        let message = format!("{err:#}");
        assert!(
            message.contains("obs-text"),
            "expected obs-text refusal, got {message}"
        );
        assert!(
            !message.contains("tab"),
            "OWS is not obs-text: {message}"
        );
    }
}
