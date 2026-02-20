/**
 * Integration token decoding utilities.
 *
 * The Hawk backend identifies projects via an integration token which is a
 * base64-encoded JSON string containing (at minimum) an `integrationId` field
 * and a `secret` field.
 *
 * The flow matches the Node.js catcher exactly:
 * 1. Receive the raw base64 token string from the user.
 * 2. Base64-decode it into a UTF-8 JSON string.
 * 3. Parse the JSON to extract `integrationId`.
 * 4. Build the default collector endpoint: `https://{integrationId}.k1.hawk.so/`
 *
 * If the user provides a custom `collector_endpoint`, this decoding is still
 * performed for validation — but the custom endpoint takes precedence.
 */
use base64::Engine as _;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// DecodedToken — the parsed contents of a base64 integration token
// ---------------------------------------------------------------------------

/**
 * Represents the decoded contents of a Hawk integration token.
 *
 * The token is base64-encoded JSON that looks like:
 * ```json
 * {
 *   "integrationId": "abc123...",
 *   "secret": "xyz789..."
 * }
 * ```
 *
 * We only need `integrationId` to derive the default collector endpoint.
 * The `secret` field is present in the token but not used by the SDK directly.
 */
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodedToken {
    /// The project's unique integration identifier used to route events.
    pub integration_id: String,

    /// Secret hash (present in the token, unused by the SDK at runtime).
    #[allow(dead_code)]
    pub secret: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Decodes a base64-encoded integration token into its structured form.
 *
 * # Arguments
 * * `token` — The raw base64-encoded integration token string provided
 *   by the user (obtained from the Hawk project settings page).
 *
 * # Returns
 * * `Ok(DecodedToken)` containing the parsed integration ID and secret.
 * * `Err(String)` with a human-readable message if decoding or parsing fails.
 *
 * # Example
 * ```ignore
 * let decoded = decode_token("eyJpbnRlZ3JhdGlvbklkIjoiYWJjIiwic2VjcmV0IjoieHl6In0=")?;
 * assert_eq!(decoded.integration_id, "abc");
 * ```
 */
pub fn decode_token(token: &str) -> Result<DecodedToken, String> {
    /*
     * Step 1: Base64 decode the token into raw bytes.
     * We use the STANDARD engine which handles the normal base64 alphabet
     * (A-Z, a-z, 0-9, +, /) with optional `=` padding — matching Node.js
     * `Buffer.from(token, 'base64')` behaviour.
     */
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(token)
        .map_err(|e| format!("Failed to base64-decode integration token: {e}"))?;

    /*
     * Step 2: Parse the decoded bytes directly as JSON.
     * `from_slice` handles UTF-8 validation internally, avoiding
     * an intermediate String allocation.
     */
    let decoded: DecodedToken = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Failed to parse integration token: {e}"))?;

    /*
     * Step 4: Validate that the integration ID is not empty — same check
     * as the Node.js catcher performs.
     */
    if decoded.integration_id.is_empty() {
        return Err("Invalid integration token: integrationId is empty".into());
    }

    Ok(decoded)
}

/**
 * Builds the default collector endpoint URL from an integration ID.
 *
 * The format matches the Node.js catcher:
 * `https://{integrationId}.k1.hawk.so/`
 *
 * # Arguments
 * * `integration_id` — The integration ID extracted from the decoded token.
 *
 * # Returns
 * The full collector URL as a `String`.
 */
pub fn default_endpoint(integration_id: &str) -> String {
    format!("https://{integration_id}.k1.hawk.so/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /**
     * Verifies the full round-trip: encode a known JSON payload as base64,
     * then decode it and check that we recover the integration ID.
     */
    #[test]
    fn test_decode_valid_token() {
        /* Build a base64-encoded token from a known JSON object */
        let json = r#"{"integrationId":"test123","secret":"s3cret"}"#;
        let token = base64::engine::general_purpose::STANDARD.encode(json);

        let decoded = decode_token(&token).expect("should decode successfully");
        assert_eq!(decoded.integration_id, "test123");
        assert_eq!(decoded.secret, "s3cret");
    }

    /**
     * Verifies that a garbage (non-base64) string produces a clear error.
     */
    #[test]
    fn test_decode_invalid_base64() {
        let result = decode_token("not-valid-base64!!!");
        assert!(result.is_err());
    }

    /**
     * Verifies that a valid base64 string containing non-JSON content fails.
     */
    #[test]
    fn test_decode_invalid_json() {
        let token = base64::engine::general_purpose::STANDARD.encode("not json");
        let result = decode_token(&token);
        assert!(result.is_err());
    }

    /**
     * Verifies that an empty integrationId is rejected.
     */
    #[test]
    fn test_decode_empty_integration_id() {
        let json = r#"{"integrationId":"","secret":"s3cret"}"#;
        let token = base64::engine::general_purpose::STANDARD.encode(json);
        let result = decode_token(&token);
        assert!(result.is_err());
    }

    /**
     * Verifies the default endpoint URL format.
     */
    #[test]
    fn test_default_endpoint() {
        assert_eq!(
            default_endpoint("abc123"),
            "https://abc123.k1.hawk.so/"
        );
    }
}
