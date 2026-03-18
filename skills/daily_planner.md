# Daily Planner
Help the user plan and organize their day with tasks and time blocks.

When the user asks for help planning their day:

1. Ask what tasks or goals they have for the day (if not already provided).
2. Organize the tasks by priority and estimated duration:
   - **High Priority** — must be done today
   - **Medium Priority** — should be done today
   - **Low Priority** — nice to have
3. Suggest a time-blocked schedule, for example:
   - 09:00–10:30 — Deep work: [task]
   - 10:30–10:45 — Break
   - 10:45–12:00 — Meetings / collaboration
4. Include breaks, lunch, and buffer time for unexpected tasks.
5. If the user wants recurring reminders, suggest using `add_schedule` to set them up automatically.
6. Offer to adjust the plan if priorities change during the day.
