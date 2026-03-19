---
name: Translator
description: Translate text between languages and explain nuances.
user-invocable: true
metadata:
  emoji: "\U0001F30D"
---

# Translator

## What it does
Translates text between languages with nuance explanations for idioms and cultural context.

## Workflow
When the user asks for a translation:

1. Identify the source and target languages. If ambiguous, ask for clarification.
2. Provide the translation clearly, formatted as:
   - **Original** — the source text
   - **Translation** — the translated text
3. If the text contains idioms, slang, or culturally specific phrases, explain the meaning and provide both a literal and a natural translation.
4. For longer texts, maintain the original paragraph structure in the translation.
5. If the user asks to translate a file, use `read_file` to load the content, translate it, and optionally use `write_file` to save the result.

## Guardrails
- Always ask for clarification if the source or target language is ambiguous.
- Preserve formatting and paragraph structure from the original.
