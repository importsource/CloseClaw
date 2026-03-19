---
name: Code Review
description: Review code for bugs, style issues, and improvements.
user-invocable: true
metadata:
  emoji: "\U0001F50D"
---

# Code Review

## What it does
Reviews code files for bugs, style issues, security vulnerabilities, and potential improvements.

## Workflow
When the user asks you to review code, follow this process:

1. Read the file(s) the user specifies.
2. Check for common issues:
   - Potential bugs or logic errors
   - Missing error handling
   - Security vulnerabilities (injection, hardcoded secrets, etc.)
   - Performance concerns
   - Code style and readability
3. Provide feedback grouped by severity:
   - **Critical** — bugs or security issues that must be fixed
   - **Warning** — potential problems or bad practices
   - **Suggestion** — optional improvements for readability or performance
4. For each finding, quote the relevant code and explain the issue concisely.

## Guardrails
- Do not modify files unless the user explicitly asks for fixes.
- Always quote the specific lines you are referencing.
- Be constructive — explain *why* something is a problem, not just *that* it is.
