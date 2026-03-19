# LinkedIn Auto-Apply Skill

A built-in skill (`skills/linkedin_apply.md`) that teaches the agent to search LinkedIn jobs, match them against your resume, and apply via Easy Apply — fully autonomously.

## Prerequisites

Complete the [Browser Tool setup](browser-tool.md) first.

## Setup

### 1. Create your candidate profile

Create a `candidate_profile.toml` in the workspace root:

```toml
# candidate_profile.toml

[personal]
first_name = "Jane"
last_name = "Doe"
full_name = "Jane Doe"
email = "jane@example.com"
phone = "5551234567"
phone_country = "United States (+1)"
city = "San Francisco, CA"
state = "California"
zip = "94105"
website = "https://janedoe.dev"
linkedin = "https://www.linkedin.com/in/janedoe/"

[work]
current_company = "Acme Corp"
current_title = "Senior Software Engineer"
years_of_experience = 8
work_authorization = "Yes"
sponsorship_required = "No"
willing_to_relocate = "Yes"
remote_ok = "Yes"
start_date = "Immediately"
salary_expectation = 180000

[resume]
# Path relative to workspace root
file = "Resume-Jane.pdf"

[cover_letter]
default = "Your brief cover letter / elevator pitch here."

[skills]
core = ["Python", "Go", "Kubernetes", "AWS", "PostgreSQL"]
ai = ["LLM", "RAG", "Fine-tuning"]
languages = ["Python", "Go", "SQL"]

[target_roles]
titles = ["Senior Software Engineer", "Staff Engineer", "Backend Engineer"]
keywords_match = ["Python", "Go", "distributed systems", "microservices", "cloud"]
keywords_skip = ["frontend-only", "iOS", "Android", "security clearance"]
min_years_acceptable = 5
location = "San Francisco"

[demographics]
gender = "Prefer not to say"
race = "Decline to self-identify"
veteran = "Prefer not to say"
disability = "Prefer not to say"

[defaults]
how_heard = "LinkedIn"
```

### 2. Place your resume PDF in the workspace root

```bash
cp /path/to/your/Resume.pdf ./Resume-Jane.pdf
```

Make sure the filename matches `[resume].file` in your profile.

### 3. Increase max iterations

LinkedIn apply needs many tool calls. In your `config.toml`:

```toml
[llm]
max_iterations = 100   # default 25 is too low for job applications
```

## Usage

### First run — log in to LinkedIn

1. Start CloseClaw: `./target/release/closeclaw run`
2. Tell the agent: **"Launch the browser"**
3. Edge/Chrome opens with a clean profile. **Log in to LinkedIn manually** in the browser window.
4. Your session is saved in `.browser-profile/` — you only need to log in once.

### Apply for jobs

Tell the agent (via Telegram, WebChat, or CLI):

> "Search for software engineer jobs in San Francisco and apply to matching ones"

The agent will:

1. Read your `candidate_profile.toml`
2. Navigate to LinkedIn job search
3. Extract all job listings
4. For each job:
   - Read the job description
   - Match against your `keywords_match` / `keywords_skip`
   - If good match + Easy Apply available → fill the form and submit
   - If poor match or external site → skip
5. Send you a single summary at the end:
   - **Applied:** list of jobs with titles and companies
   - **Skipped:** list with reasons
   - **Total:** X applied, Y skipped out of Z found

Screenshots are taken after each submission and sent automatically if you're using Telegram.

## How matching works

The agent reads the job description and checks it against your profile:

- **Match keywords** (`keywords_match`): If the job mentions any of these, it's a positive signal.
- **Skip keywords** (`keywords_skip`): If the job is clearly focused on one of these (e.g. "frontend-only"), it gets skipped.
- **Target titles** (`titles`): The agent considers how well the job title aligns with your target roles.
- **Years of experience** (`min_years_acceptable`): Jobs requiring significantly more experience than you have may be skipped.

The agent uses its judgment to weigh all these factors — it's not a simple keyword filter.

## How form filling works

When the agent encounters an Easy Apply form:

1. It inspects all form fields (inputs, selects, textareas)
2. **Pre-filled fields are skipped** — LinkedIn often pre-fills name, email, phone from your profile
3. Empty fields are filled using data from your `candidate_profile.toml`:
   - Personal info → `[personal]`
   - Work authorization, sponsorship → `[work]`
   - Resume upload → `[resume].file`
   - Cover letter → `[cover_letter].default`
   - Demographics / EEO → `[demographics]`
   - "How did you hear about us?" → `[defaults].how_heard`
4. For unknown questions, the agent picks the most reasonable answer based on your profile
5. It clicks Next/Continue/Submit automatically through each step

## Important notes

- The agent runs **fully autonomously** — it will not ask for confirmation between jobs.
- The **only** time it stops is if it hits a CAPTCHA or security check.
- `candidate_profile.toml` and `Resume-*.pdf` are gitignored — your personal data stays local.
- The `.browser-profile/` directory is also gitignored.

## Troubleshooting

| Problem | Fix |
|---------|-----|
| Agent runs out of iterations | Set `max_iterations = 100` (or higher) in `config.toml` |
| LinkedIn shows "Sign in" | Your session expired. Tell the agent to launch the browser, log in manually, then retry. |
| Form fields not filling | Check that `candidate_profile.toml` has the relevant fields. The agent uses best judgment for unknown fields. |
| Agent keeps asking for confirmation | This shouldn't happen — the skill explicitly forbids it. If it does, try increasing `max_iterations`. |
| Jobs not matching well | Tune `keywords_match` and `keywords_skip` in your profile. More specific keywords = better matching. |
