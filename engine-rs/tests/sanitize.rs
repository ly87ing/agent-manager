//! 错误消息脱敏：凭据、私钥、URL userinfo 与敏感查询参数在出库前一律掩码，
//! 错误因果链保持可读。

use agent_manager_engine::sanitize::sanitize_diagnostic_message;

#[test]
fn centralized_redaction_masks_credentials_and_preserves_error_chain() {
    // 1. 私钥内容屏蔽
    let privkey = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA0abc123...\n-----END RSA PRIVATE KEY-----";
    let masked_key = sanitize_diagnostic_message(&format!("Key loading failed: {privkey}"));
    assert!(!masked_key.contains("MIIEowIBAAKCAQEA0abc123"));
    assert!(masked_key.contains("[REDACTED PRIVATE KEY]"));

    // 2. Bearer 令牌屏蔽
    let bearer_str = "Authorization: Bearer my-super-secret-token-xyz123";
    let masked_bearer = sanitize_diagnostic_message(bearer_str);
    assert!(!masked_bearer.contains("my-super-secret-token-xyz123"));
    assert!(masked_bearer.contains("Bearer ***"));

    // 3. URL user:pass 屏蔽
    let url_str = "Failed to clone https://bot_user:secret_pwd_999@gitlab.example.com/repo.git";
    let masked_url = sanitize_diagnostic_message(url_str);
    assert!(!masked_url.contains("secret_pwd_999"));
    assert!(masked_url.contains("https://bot_user:***@gitlab.example.com/repo.git"));

    // 4. URL query 参数屏蔽
    let query_str = "Request failed: https://api.service.com/v1/sync?token=tok_secret_888&api_key=ak_secret_777&debug=true";
    let masked_query = sanitize_diagnostic_message(query_str);
    assert!(!masked_query.contains("tok_secret_888"));
    assert!(!masked_query.contains("ak_secret_777"));
    assert!(masked_query.contains("token=***"));
    assert!(masked_query.contains("api_key=***"));
    assert!(masked_query.contains("debug=true"));

    // 5. 常见键值对屏蔽 (双引号带空格、单引号带空格、无引号值)
    let kv_str = "Config invalid: password=\"top secret value with spaces\", token='single quoted secret phrase', api_key: bare_secret_val_123, other=\"normal value\"";
    let masked_kv = sanitize_diagnostic_message(kv_str);
    assert!(!masked_kv.contains("top secret value with spaces"));
    assert!(!masked_kv.contains("secret value"));
    assert!(masked_kv.contains("password=\"***\""));
    assert!(!masked_kv.contains("single quoted secret phrase"));
    assert!(!masked_kv.contains("secret phrase"));
    assert!(masked_kv.contains("token='***'"));
    assert!(!masked_kv.contains("bare_secret_val_123"));
    assert!(masked_kv.contains("api_key: ***"));
    assert!(masked_kv.contains("other=\"normal value\""));

    // 6. 知名 token 模式 (GitHub / OpenAI)
    let tokens_str =
        "Found tokens ghp_1234567890abcdef1234567890abcdef1234 and sk-123456789012345678901234";
    let masked_tokens = sanitize_diagnostic_message(tokens_str);
    assert!(!masked_tokens.contains("1234567890abcdef"));
    assert!(masked_tokens.contains("gh***"));
    assert!(masked_tokens.contains("sk-***"));

    // 7. anyhow 错误链保留测试
    let root_cause = anyhow::anyhow!("HTTP 401: Bearer token_leak_123 rejected");
    let wrap_err = anyhow::anyhow!("upstream sync failed: {root_cause:#}");
    let formatted = format!("{wrap_err:#}");
    let sanitized_chain = sanitize_diagnostic_message(&formatted);
    assert!(sanitized_chain.starts_with("upstream sync failed: HTTP 401: Bearer *** rejected"));
    assert!(!sanitized_chain.contains("token_leak_123"));
}
