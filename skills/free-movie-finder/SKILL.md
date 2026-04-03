---
name: Free Movie Finder
description: Find free English movies without Chinese captions across streaming platforms.
user-invocable: true
metadata:
  emoji: "\U0001F3AC"
---

# Free Movie Finder

## What it does
Searches the web for free, legally streamable English movies that do not have hardcoded Chinese subtitles or captions.

## Workflow
When the user invokes this skill (optionally with a genre, era, or other preference):

1. Use `web_search` with targeted queries to find free full-length English movies. Example queries:
   - `free full movie English site:youtube.com`
   - `free English movies no subtitles site:archive.org`
   - `free movies English Tubi`
   - `free movies English Pluto TV`
   If the user specifies a genre or era, include it in the query (e.g. `free full movie English horror 1980s`).
   Add exclusion terms to filter out Chinese-captioned results: `-Chinese -中文 -字幕 -双语`.
2. Use `web_fetch` on the top 8-10 results to verify each movie:
   - Is actually free to watch (no paywall, no rental-only).
   - Has English audio.
   - Does not have hardcoded Chinese subtitles burned into the video.
3. Present a curated list of 5-10 movies in this format:
   - **Title** (Year) — Genre
   - One-line description
   - **Watch:** direct link to the free stream
   - **Source:** platform name (YouTube, Archive.org, Tubi, Pluto TV, Plex, etc.)
4. If fewer than 5 good results are found, run additional searches with alternate queries or platforms.

## Prioritized Sources
Search these platforms first — they are known for free, legal, English-language content:
- **YouTube Movies** — free with ads, large catalog
- **Archive.org** — public domain classics
- **Tubi** — free ad-supported streaming
- **Pluto TV** — free live and on-demand
- **Plex Free** — free ad-supported movies

## Guardrails
- Only recommend movies that are legally free to watch.
- Exclude any result that has hardcoded Chinese subtitles or is primarily a Chinese-dubbed version.
- Always provide a direct watch link — do not link to search result pages.
- If a result is region-locked, note that in the listing.
- Cite the platform and URL for every recommendation.
