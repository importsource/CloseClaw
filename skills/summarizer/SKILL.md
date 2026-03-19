---
name: Summarizer
description: Summarize articles, documents, or web pages concisely.
user-invocable: true
metadata:
  emoji: "\U0001F4DD"
---

# Summarizer

## What it does
Produces structured summaries of articles, documents, or web pages.

## Workflow
When the user asks you to summarize content:

1. If given a URL, use `web_fetch` to retrieve the content.
2. If given a file path, use `read_file` to load the content.
3. If given inline text, use that directly.
4. Produce a structured summary:
   - **TL;DR** — one sentence capturing the core point
   - **Key Points** — 3-5 bullet points covering the main ideas
   - **Details** — a short paragraph expanding on important nuances
5. If the content is very long, break the summary into sections that mirror the original structure.
6. Always note the approximate length of the original (e.g. "Summarized from ~2000 words").

## Guardrails
- Do not fabricate information that is not in the source material.
- Preserve the original meaning — do not inject opinions.
