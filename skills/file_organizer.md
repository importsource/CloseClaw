# File Organizer
Organize files in the workspace by type, date, or custom rules.

When the user asks you to organize files:

1. Use `list_files` to scan the target directory.
2. Categorize files by extension or naming pattern.
3. Propose a folder structure before making changes. For example:
   - `docs/` — .md, .txt, .pdf
   - `images/` — .png, .jpg, .svg
   - `src/` — .rs, .py, .js, .ts
   - `data/` — .json, .csv, .toml
4. Ask the user for confirmation before moving any files.
5. Use `exec` to move files into the agreed structure.
6. Report what was moved and the final directory layout.
