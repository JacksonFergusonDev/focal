#!/usr/bin/env bats

setup() {
    WEB_BIN="${BATS_TEST_DIRNAME}/../../libexec/web"
}

@test "web -h prints usage and options" {
    run "$WEB_BIN" -h

    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: focal web"* ]]
    [[ "$output" == *"Extracts semantic markdown from public URLs or authenticated piped HTML."* ]]
}

@test "web bare execution with empty input fails with hint" {
    run bash -c "'$WEB_BIN' </dev/null"

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: received empty piped input"* ]]
    [[ "$output" == *"hint: pipe HTML via stdin or provide a URL argument"* ]]
}

@test "web rejects whitespace-only piped input" {
    run bash -c "printf '   \n  ' | '$WEB_BIN'"

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: received empty piped input"* ]]
    [[ "$output" == *"hint: pipe HTML via stdin or provide a URL argument"* ]]
}

@test "web converts piped HTML and excludes noise tags" {
    html='<html><head><script>console.log("bad")</script></head><body><header>head</header><nav>nav</nav><main><h1>Hello</h1><p>World</p><button>click</button></main><footer>foot</footer></body></html>'

    run bash -c "printf '%s' '$html' | '$WEB_BIN'"

    [ "$status" -eq 0 ]
    [[ "$output" == *"# Source: Piped DOM/Clipboard"* ]]
    [[ "$output" == *"# Hello"* ]]
    [[ "$output" == *"World"* ]]
    [[ "$output" != *"bad"* ]]
    [[ "$output" != *"click"* ]]
    [[ "$output" != *"foot"* ]]
}

@test "web converts public URL" {
    run "$WEB_BIN" https://example.com

    [ "$status" -eq 0 ]
    [[ "$output" == *"# Source: https://example.com"* ]]
    [[ "$output" == *"Example Domain"* ]]
}

@test "web normalizes bare domain to https" {
    run "$WEB_BIN" example.com

    [ "$status" -eq 0 ]
    [[ "$output" == *"# Source: https://example.com"* ]]
    [[ "$output" == *"Example Domain"* ]]
}

@test "web rejects invalid URL scheme" {
    run "$WEB_BIN" file:///etc/passwd

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: Invalid URL scheme in 'file:///etc/passwd'"* ]]
    [[ "$output" == *"hint: URL must begin with http:// or https://"* ]]
}

@test "web truncates content exceeding MAX_OUTPUT_CHARS" {
    large_html=$(python3 -c "print('<p>' + 'A' * 60000 + '</p>')")

    run bash -c "printf '%s' '$large_html' | '$WEB_BIN'"

    [ "$status" -eq 0 ]
    [[ "$output" == *"...[web content truncated: exceeded 50000 characters]"* ]]
}

@test "web surfaces anti-bot hint on HTTP 999" {
    run "$WEB_BIN" https://www.linkedin.com/in/jackson--ferguson/

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: HTTP 999 returned by www.linkedin.com"* ]]
    [[ "$output" == *"hint: this site blocks automated CLI scrapers. Open the page in your browser and run: pbpaste | focal web"* ]]
}

@test "web surfaces not found hint on HTTP 404" {
    run "$WEB_BIN" https://httpbin.org/status/404

    [ "$status" -eq 1 ]
    [[ "$output" == *"error: HTTP 404 Not Found"* ]]
    [[ "$output" == *"hint: check that the URL path is spelled correctly"* ]]
}
