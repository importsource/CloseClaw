---
name: News Briefing
description: Search the web and summarize recent news on a topic.
user-invocable: true
metadata:
  emoji: "\U0001F4F0"
---

# News Briefing

## What it does
Searches the web for recent news articles on a topic and delivers a structured briefing.

## Workflow
When the user asks for a news briefing:

1. Use the `web_search` tool to find recent articles on the requested topic.
2. Fetch the top 3-5 results using `web_fetch` to get article content.
3. Summarize each article in 2-3 sentences, noting the source and date.
4. Present a structured briefing:
   - **Overview** — one paragraph summary of the current state
   - **Key Stories** — bullet list of individual article summaries
   - **Sources** — links to the original articles

## Guardrails
- Always cite sources with URLs.
- Distinguish between reported facts and editorial opinions.
- Note the date of each source to help the user gauge freshness.
