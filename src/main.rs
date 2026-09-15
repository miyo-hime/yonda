use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use ureq::{Agent, ResponseExt};

const UA: &str = concat!("linux:yonda:v", env!("CARGO_PKG_VERSION"), " (personal thread reader)");
const USAGE: &str = "usage: yonda <reddit-thread-url> [--sort best|top|new|controversial|old|qa] [--limit N]";

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
    let mut args = std::env::args().skip(1);
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
    let agent: Agent = Agent::config_builder().user_agent(UA).http_status_as_error(false).build().into();
    let text = fetch(&agent, &cookie, &url, sort.as_deref(), limit)?;

    let (post_listing, comments): (Listing, Listing) = serde_json::from_str(&text).context("unexpected response shape - is this a thread url?")?;
    let Some(Node::Post(post)) = post_listing.data.children.into_iter().next() else { bail!("no post in response") };

    let mut out = String::new();
    render_post(&post, &mut out);
    for node in &comments.data.children { render_node(node, 0, &mut out); }
    print!("{out}");
    Ok(())
}

fn session_cookie() -> Result<String> {
    let home = std::env::var("HOME").context("no $HOME")?;
    let path = format!("{home}/.config/yonda/session");
    let raw = std::fs::read_to_string(&path).with_context(|| format!("couldn't read {path} - put your reddit_session cookie value there"))?;
    Ok(raw.trim().to_owned())
}

fn fetch(agent: &Agent, cookie: &str, raw_url: &str, sort: Option<&str>, limit: u32) -> Result<String> {
    let tail = raw_url.find("reddit.com").map(|i| &raw_url[i + 10..]).ok_or_else(|| anyhow!("not a reddit url: {raw_url}"))?;
    let path = tail.split(['?', '#']).next().unwrap_or("").trim_end_matches('/');
    let path = if path.contains("/s/") { resolve_share(agent, cookie, path)? } else { path.to_owned() };

    // ※ without raw_json=1 reddit html-escapes every body (&amp; &lt;) - markdown would render the entities
    let mut url = format!("https://www.reddit.com{path}.json?raw_json=1&limit={limit}");
    if let Some(s) = sort { let s = if s == "best" { "confidence" } else { s }; write!(url, "&sort={s}").unwrap(); }

    let mut res = agent.get(&url).header("Cookie", format!("reddit_session={cookie}")).call()?;
    match res.status().as_u16() {
        200 => Ok(res.body_mut().read_to_string()?),
        403 => bail!("reddit said 403 - the session cookie is probably stale, re-paste it from the browser"),
        429 => bail!("reddit said 429 - rate limited, wait a minute"),
        code => bail!("reddit said {code} for {url}"),
    }
}

fn resolve_share(agent: &Agent, cookie: &str, path: &str) -> Result<String> {
    let res = agent.get(format!("https://www.reddit.com{path}")).header("Cookie", format!("reddit_session={cookie}")).call()?;
    let uri = res.get_uri();
    Ok(uri.path().trim_end_matches('/').to_owned())
}

fn render_post(p: &Post, out: &mut String) {
    let _ = writeln!(out, "# {}\n", p.title);
    let _ = write!(out, "**r/{}** · u/{} · {} ({:.0}% upvoted) · {} comments · {}", p.subreddit, p.author, points(p.score), p.upvote_ratio * 100.0, p.num_comments, ago(p.created_utc));
    if let Some(flair) = p.link_flair_text.as_deref().filter(|f| !f.is_empty()) { let _ = write!(out, " · [{flair}]"); }
    if p.over_18 { let _ = write!(out, " · **NSFW**"); }
    out.push_str("\n\n");
    if !p.selftext.trim().is_empty() { let _ = writeln!(out, "{}\n", p.selftext.trim()); }
    else if !p.is_self { let _ = writeln!(out, "<{}>\n", p.url); }
    out.push_str("---\n\n");
}

fn render_node(node: &Node, depth: usize, out: &mut String) {
    let q = "> ".repeat(depth);
    match node {
        Node::Comment(c) => {
            let _ = write!(out, "{q}**u/{}**", c.author);
            if c.is_submitter { out.push_str(" (OP)"); }
            if c.stickied { out.push_str(" 📌"); }
            let _ = writeln!(out, " · {} · {}", points(c.score), ago(c.created_utc));
            let _ = writeln!(out, "{}", q.trim_end());
            for line in c.body.trim().lines() { let _ = writeln!(out, "{q}{line}"); }
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
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let d = (now - utc as i64).max(0);
    match d {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{}m ago", d / 60),
        3600..86400 => format!("{}h ago", d / 3600),
        86400..2592000 => format!("{}d ago", d / 86400),
        2592000..31536000 => format!("{}mo ago", d / 2592000),
        _ => format!("{}y ago", d / 31536000),
    }
}
