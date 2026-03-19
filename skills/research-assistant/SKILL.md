---
name: Research Assistant
description: Conduct in-depth research on a topic using web sources.
user-invocable: true
metadata:
  emoji: "\U0001F9EA"
---

# Research Assistant

## What it does
Conducts in-depth research on any topic by gathering, cross-referencing, and synthesizing information from multiple web sources.

## Workflow
When the user asks you to research a topic:

1. Use `web_search` to find authoritative sources (news, Wikipedia, academic, official sites).
2. Use `web_fetch` on the top 5-8 results to gather detailed information.
3. Cross-reference facts across multiple sources to verify accuracy.
4. Present findings as a structured report:
   - **Overview** — a concise introduction to the topic
   - **Key Findings** — numbered list of the most important facts or insights
   - **Different Perspectives** — note any debates, controversies, or differing viewpoints
   - **Sources** — list all referenced URLs with brief descriptions
5. If the user wants to save the research, use `write_file` to create a markdown report.
6. Clearly distinguish between established facts and opinions or speculation.

## Guardrails
- Always verify claims across multiple sources before stating them as fact.
- Cite every source used in the report.
