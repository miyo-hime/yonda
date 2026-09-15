---
name: yonda
description: Read reddit from an agent session. Use when a reddit URL needs opening (a thread, a comment chain, subreddit recon), always use first before attempting to fetch reddit.com.
---

# yonda - the reddit door map

reddit blocks agents at most entrances: WebFetch, anonymous `.json`, redlib/teddit mirrors, and old.reddit. Skip all of those. The doors that open, in order:

## a thread (the main case)

```
yonda <url> [--sort best|top|new|controversial|old|qa] [--limit N]
```

Prints the post and full comment tree as markdown on stdout. Takes www, old.reddit, and share (`/s/`) links. Output can be thousands of lines - pipe through `head` or into a file when the thread is big.

- `403` means the session cookie went stale: ask the human to re-paste it (setup lives in the repo README, `~/.config/yonda/session`).
- reddit 429s a second request within a few seconds. One call per thread, and let results earn their keep before fetching another.

## a subreddit's current front page

The RSS door needs no auth: `curl -A "<any-ua>" "https://www.reddit.com/r/<sub>/.rss"` gives titles and links, lossy but live. Same rate bouncer applies.

## searching reddit

No direct door. Use a web search engine with `site:reddit.com`, then open the winning thread with yonda.

## everything else

A page yonda can't express (wiki pages, user profiles, JS-walled corners): drive a real browser (playwright) as the last resort.

## the write side

yonda is read-only by design - the binary contains no POST. Anything that writes to reddit (posting, voting, replying) is the human's decision, made by the human, in a browser.
