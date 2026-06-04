use super::*;

#[test]
fn in_scope_matches_origin() {
    assert!(url_in_scope(
        "http://target.local:32918/path?q=1",
        "http://target.local:32918"
    ));
    assert!(url_in_scope(
        "https://target.local:443/",
        "https://target.local:443"
    ));
}

#[test]
fn out_of_scope_origins_rejected() {
    let scope = "http://target.local:32918";
    assert!(!url_in_scope("http://127.0.0.1:32918/", scope));
    assert!(!url_in_scope(
        "http://169.254.169.254/latest/meta-data/",
        scope
    ));
    assert!(
        !url_in_scope("http://target.local:80/", scope),
        "wrong port"
    );
    assert!(
        !url_in_scope("https://target.local:32918/", scope),
        "wrong scheme"
    );
    assert!(!url_in_scope("http://attacker.example/", scope));
}

#[test]
fn dangerous_schemes_always_rejected() {
    let scope = "http://target.local:32918";
    assert!(!url_in_scope("file:///etc/passwd", scope));
    assert!(!url_in_scope("javascript:alert(1)", scope));
    assert!(!url_in_scope("data:text/html,<script>", scope));
}
