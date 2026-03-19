---
name: Daily Planner
description: Help the user plan and organize their day with tasks and time blocks.
user-invocable: true
metadata:
  emoji: "\U0001F4C5"
---

# Daily Planner

## What it does
Plans and organizes the user's day with prioritized tasks and time-blocked schedules.

## Workflow
When the user asks for help planning their day:

1. Ask what tasks or goals they have for the day (if not already provided).
2. Organize the tasks by priority and estimated duration:
   - **High Priority** — must be done today
   - **Medium Priority** — should be done today
   - **Low Priority** — nice to have
3. Suggest a time-blocked schedule, for example:
   - 09:00-10:30 — Deep work: [task]
   - 10:30-10:45 — Break
   - 10:45-12:00 — Meetings / collaboration
4. Include breaks, lunch, and buffer time for unexpected tasks.
5. If the user wants recurring reminders, suggest using `add_schedule` to set them up automatically.
6. Offer to adjust the plan if priorities change during the day.

## Guardrails
- Respect the user's stated availability and constraints.
- Always include breaks — avoid overloading the schedule.
