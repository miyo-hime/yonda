# yonda

reads a reddit thread and prints it as markdown in your terminal. that's it, really. made it so i could point and laugh at reddit with my AI agents.

読んだ - "read it". also 呼んだ - "you called?"

```
yonda https://www.reddit.com/r/rust/comments/xxxxxx/some_thread/
```

you get the post (title, sub, author, score, selftext) followed by the full comment tree as nested blockquotes. pipe it into `glow`, `bat`, or `less` and read.

## why it uses your session cookie

because `reddit.com/prefs/apps` is a zombie. still renders, but creating an app does nothing. data API access now goes through an approval form, and from what i can tell, individual devs mostly get rejected. the anonymous `.json` endpoints are also blocked from most networks now (403 with a block page).

what still works: the same logged-in session your browser uses. so yonda borrows it. no OAuth, no client id, no waiting for approval - you're just reading reddit as yourself, from a terminal instead of a tab.

is this against ToS? i don't know lol (and don't care). but it's your own account reading threads you could read in a browser. use your own judgement and don't run it in a loop.

## setup

```
cargo install --path .
```

then give it your session cookie, once:

1. open reddit in your browser, logged in
2. F12 → Application (chrome) or Storage (firefox) → Cookies → `https://www.reddit.com`
3. copy the value of `reddit_session`
4. paste it into `~/.config/yonda/session` (just the raw value)
5. `chmod 600 ~/.config/yonda/session`

the cookie lives until you log out in that browser, so this is close to a one-time thing. if yonda starts answering 403, re-paste it.

## usage

```
yonda <thread-url> [--sort best|top|new|controversial|old|qa] [--limit N]
```

share links (`/s/`) and old.reddit urls work too. `--limit` caps how many comments reddit returns (default 500); deeper cuts show up as `(+N more not fetched)`.

## what it doesn't do

post, comment, vote, or write anything. there is no POST in the binary.

not yet anyway.

also no caching yet. reddit rate-limits harder than you'd expect (a second request within a few seconds can draw a 429), so be gentle~ 💕
