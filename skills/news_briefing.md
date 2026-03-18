# News Briefing
Search the web and summarize recent news on a topic.

When the user asks for a news briefing:

1. Use the `web_search` tool to find recent articles on the requested topic.
2. Fetch the top 3-5 results using `web_fetch` to get article content.
3. Summarize each article in 2-3 sentences, noting the source and date.
4. Present a structured briefing:
   - **Overview** — one paragraph summary of the current state
   - **Key Stories** — bullet list of individual article summaries
   - **Sources** — links to the original articles
