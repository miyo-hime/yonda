use std::fmt::Write as _;
use std::io::{ErrorKind, Read as _, Write as _};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use ureq::Agent;

const UA: &str = concat!("linux:yonda:v", env!("CARGO_PKG_VERSION"), " (personal thread reader)");
const USAGE: &str = "usage: yonda <reddit-thread-url> [--sort best|top|new|controversial|old|qa] [--limit N]";
const THREAD_URL_HINT: &str = "expected a reddit thread url like https://www.reddit.com/r/rust/comments/abc123/title/";
const MAX_RESPONSE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Deserialize)]
struct Listing { data: ListingData }

#[derive(Deserialize)]
struct ListingData { children: Vec<Node> }

#[derive(Deserialize)]
#[serde(tag = "kind", content = "data")]
enum Node {
    #[serde(rename = "t3")] Post(Post),
    #[serde(rename = "t1")] Comment(Comment),
    #[serde(rename = "more")] More(More),
    #[serde(other)] Other,
}

#[derive(Deserialize)]
struct Post {
    title: String, author: String, subreddit: String, selftext: String, url: String,
    score: i64, upvote_ratio: f64, num_comments: i64, created_utc: f64, is_self: bool,
    #[serde(default)] link_flair_text: Option<String>,
    #[serde(default)] over_18: bool,
}

#[derive(Deserialize)]
struct Comment {
    author: String, body: String, score: i64, created_utc: f64,
    #[serde(default)] is_submitter: bool,
    #[serde(default)] stickied: bool,
    #[serde(default)] replies: Replies,
}

// ※ reddit sends replies: "" on leaf comments instead of null - the string variant absorbs it
#[derive(Deserialize, Default)]
#[serde(untagged)]
enum Replies { Tree(Box<Listing>), #[allow(dead_code)] Leaf(String), #[default] Absent }

#[derive(Deserialize)]
struct More { count: i64 }

fn main() -> Result<()> {
    let (mut url, mut sort, mut limit) = (None, None::<String>, 500u32);
    let args = std::env::args_os().skip(1).map(|a| a.into_string().map_err(|_| anyhow!("invalid argument - arguments must be valid unicode"))).collect::<Result<Vec<_>>>()?;
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => { println!("{USAGE}"); return Ok(()); }
            "--sort" => sort = Some(args.next().context("--sort needs a value")?),
            "--limit" => limit = args.next().context("--limit needs a value")?.parse().context("--limit needs a number")?,
            _ if url.is_none() => url = Some(a),
            other => bail!("unexpected argument: {other}\n{USAGE}"),
        }
    }
    let url = url.ok_or_else(|| anyhow!(USAGE))?;

    let cookie = session_cookie()?;
    let agent: Agent = Agent::config_builder().user_agent(UA).http_status_as_error(false).max_redirects(0).timeout_global(Some(Duration::from_secs(30))).build().into();
    let text = fetch(&agent, &cookie, &url, sort.as_deref(), limit)?;

    let (post_listing, comments): (Listing, Listing) = serde_json::from_str(&text).context("unexpected response shape - is this a thread url?")?;
    let Some(Node::Post(post)) = post_listing.data.children.into_iter().next() else { bail!("no post in response") };

    let mut out = String::new();
    render_post(&post, &mut out);
    for node in &comments.data.children { render_node(node, 0, &mut out); }

    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    match stdout.write_all(out.as_bytes()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error).context("couldn't write stdout"),
    }
}

fn session_cookie() -> Result<String> {
    let home = std::env::var("HOME").context("no $HOME")?;
    let path = format!("{home}/.config/yonda/session");
    let raw = std::fs::read_to_string(&path).with_context(|| format!("couldn't read {path} - put your reddit_session cookie value there"))?;
    Ok(raw.trim().to_owned())
}

fn fetch(agent: &Agent, cookie: &str, raw_url: &str, sort: Option<&str>, limit: u32) -> Result<String> {
    let path = thread_path(raw_url)?;
    let path = if is_share_path(&path) { resolve_share(agent, cookie, &path)? } else { path };

    // ※ without raw_json=1 reddit html-escapes every body (&amp; &lt;) - markdown would render the entities
    let mut url = format!("https://www.reddit.com{path}.json?raw_json=1&limit={limit}");
    if let Some(s) = sort { let s = if s == "best" { "confidence" } else { s }; write!(url, "&sort={s}").unwrap(); }

    let mut res = agent.get(&url).header("Cookie", format!("reddit_session={cookie}")).call()?;
    match res.status().as_u16() {
        200 => {
            let mut bytes = Vec::new();
            res.body_mut().as_reader().take(MAX_RESPONSE_BYTES + 1).read_to_end(&mut bytes).context("couldn't read reddit response")?;
            if bytes.len() as u64 > MAX_RESPONSE_BYTES { bail!("response too large - limit is 50 MiB"); }
            String::from_utf8(bytes).context("reddit response wasn't valid utf-8")
        }
        403 => bail!("reddit said 403 - the session cookie is probably stale, re-paste it from the browser"),
        429 => bail!("reddit said 429 - rate limited, wait a minute"),
        code => bail!("reddit said {code} for {url}"),
    }
}

fn thread_path(raw_url: &str) -> Result<String> {
    let raw_url = raw_url.trim();
    let rest = if let Some((scheme, rest)) = raw_url.split_once("://") {
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") { bail!("reddit thread urls must use http or https"); }
        rest
    } else {
        raw_url.strip_prefix("//").unwrap_or(raw_url)
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() { bail!("reddit url is missing a host"); }
    if authority.contains(['@', ':']) { bail!("reddit urls may not contain userinfo or ports"); }
    if !["www.reddit.com", "old.reddit.com", "reddit.com", "np.reddit.com", "new.reddit.com"].iter().any(|host| authority.eq_ignore_ascii_case(host)) { bail!("reddit url host is not supported"); }
    let tail = &rest[authority_end..];
    let path = if tail.starts_with('/') { tail } else { "" };
    normalize_thread_path(path)
}

fn normalize_thread_path(path: &str) -> Result<String> {
    let path = path.split(['?', '#']).next().unwrap_or("").trim_end_matches('/');
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    let valid = match parts.as_slice() {
        ["r", sub, "comments", id, ..] => !sub.is_empty() && !id.is_empty(),
        ["r", sub, "s", token] => !sub.is_empty() && !token.is_empty(),
        ["comments", id, ..] => !id.is_empty(),
        _ => false,
    };
    if !valid { bail!(THREAD_URL_HINT); }
    Ok(path.to_owned())
}

fn is_share_path(path: &str) -> bool {
    matches!(path.trim_start_matches('/').split('/').collect::<Vec<_>>().as_slice(), ["r", _, "s", _])
}

fn resolve_share(agent: &Agent, cookie: &str, path: &str) -> Result<String> {
    let res = agent.get(format!("https://www.reddit.com{path}")).header("Cookie", format!("reddit_session={cookie}")).call()?;
    if !res.status().is_redirection() { bail!("reddit share link did not redirect (status {})", res.status().as_u16()); }
    let location = res.headers().get("location").context("reddit share link redirect had no location")?.to_str().context("reddit share link redirect was invalid")?;
    if location.starts_with('/') { normalize_thread_path(location) } else { thread_path(location) }
}

fn sanitize(text: &str) -> String {
    text.chars().filter(|c| matches!(*c, '\n' | '\t') || !matches!(*c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}')).collect()
}

fn render_post(p: &Post, out: &mut String) {
    let title = sanitize(&p.title);
    let subreddit = sanitize(&p.subreddit);
    let author = sanitize(&p.author);
    let _ = writeln!(out, "# {title}\n");
    let _ = write!(out, "**r/{subreddit}** · u/{author} · {} ({:.0}% upvoted) · {} comments · {}", points(p.score), p.upvote_ratio * 100.0, p.num_comments, ago(p.created_utc));
    if let Some(flair) = p.link_flair_text.as_deref() { let flair = sanitize(flair); if !flair.is_empty() { let _ = write!(out, " · [{flair}]"); } }
    if p.over_18 { let _ = write!(out, " · **NSFW**"); }
    out.push_str("\n\n");
    let selftext = sanitize(&p.selftext);
    if !selftext.trim().is_empty() { let _ = writeln!(out, "{}\n", selftext.trim()); }
    else if !p.is_self { let _ = writeln!(out, "<{}>\n", sanitize(&p.url)); }
    out.push_str("---\n\n");
}

fn render_node(node: &Node, depth: usize, out: &mut String) {
    let q = "> ".repeat(depth);
    match node {
        Node::Comment(c) => {
            let author = sanitize(&c.author);
            let body = sanitize(&c.body);
            let _ = write!(out, "{q}**u/{author}**");
            if c.is_submitter { out.push_str(" (OP)"); }
            if c.stickied { out.push_str(" 📌"); }
            let _ = writeln!(out, " · {} · {}", points(c.score), ago(c.created_utc));
            let _ = writeln!(out, "{}", q.trim_end());
            for line in body.trim().lines() { let _ = writeln!(out, "{q}{line}"); }
            out.push('\n');
            if let Replies::Tree(listing) = &c.replies {
                for child in &listing.data.children { render_node(child, depth + 1, out); }
            }
        }
        Node::More(m) if m.count > 0 => { let _ = writeln!(out, "{q}*(+{} more not fetched)*\n", m.count); }
        _ => {}
    }
}

fn points(n: i64) -> String {
    if n == 1 || n == -1 { format!("{n} point") } else { format!("{n} points") }
}

fn ago(utc: f64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs().min(i64::MAX as u64) as i64).unwrap_or(0);
    let utc = if utc.is_finite() { utc.clamp(0.0, now as f64) as i64 } else { 0 };
    let d = now.saturating_sub(utc);
    match d {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{}m ago", d / 60),
        3600..86400 => format!("{}h ago", d / 3600),
        86400..2592000 => format!("{}d ago", d / 86400),
        2592000..31536000 => format!("{}mo ago", d / 2592000),
        _ => format!("{}y ago", d / 31536000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_urls_are_rejected() {
        for url in [
            "https://www.reddit.com@attacker.example/r/x/comments/a/b/",
            "https://www.reddit.com.attacker.example/a",
            "https://example.org/?reddit.com@attacker.example/a",
            "https://www.reddit.com/",
        ] {
            assert!(thread_path(url).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn reddit_thread_urls_are_accepted() {
        assert_eq!(thread_path("https://www.reddit.com/r/rust/comments/abc123/title/").unwrap(), "/r/rust/comments/abc123/title");
        assert_eq!(thread_path("old.reddit.com/r/x/comments/y").unwrap(), "/r/x/comments/y");
    }

    #[test]
    fn hostile_timestamps_render() {
        assert!(!ago(-1e30).is_empty());
        assert!(!ago(f64::NAN).is_empty());
    }

    #[test]
    fn terminal_controls_are_sanitized() {
        assert_eq!(sanitize("keep\nthis\u{1b}and\u{9b}that"), "keep\nthisandthat");
    }
}
