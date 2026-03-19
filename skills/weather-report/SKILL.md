---
name: Weather Report
description: Fetch and present current weather and forecasts for a location.
user-invocable: true
metadata:
  emoji: "\U00002600"
---

# Weather Report

## What it does
Fetches current weather conditions and forecasts for a specified location using web search.

## Workflow
When the user asks about the weather:

1. Use `web_search` to search for the current weather at the specified location (e.g. "weather in Tokyo today").
2. Use `web_fetch` on the top result to get detailed weather data.
3. Present a clear weather report:
   - **Current Conditions** — temperature, humidity, wind speed, and a brief description (sunny, cloudy, rain, etc.)
   - **Today's Forecast** — high/low temperatures, chance of precipitation, and any weather alerts
   - **Upcoming Days** — a 3-day outlook if available
4. If the user doesn't specify a location, ask which city or region they'd like the weather for.
5. Always mention the source and time of the weather data.

## Guardrails
- Always note the data source and timestamp.
- Ask for a location if the user does not provide one.
