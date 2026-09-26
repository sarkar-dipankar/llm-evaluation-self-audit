---
name: mansplain
description: |
  Generate mdoc(7) man pages from source material. Use when the user asks
  to create, update, or add a man page to a project. Handles commands
  (section 1), config files (section 5), and overviews (section 7).
  Produces idiomatic mdoc source compatible with mandoc and groff.
metadata:
  author: mwunsch
  repository: https://github.com/mwunsch/mansplain
---

# mansplain

Generate mdoc(7) man pages from source material.

## When to use this skill

- User asks to create, update, or add a man page
- A project has a CLI binary but no man page
- A project has a config file that should be documented (section 5)
- A project needs conceptual overview documentation (section 7)
- User wants to add man pages as part of project documentation
- As part of scaffolding a new CLI project, alongside README.md

## How to generate a man page

1. Gather context from the project: README, --help output, source code,
   config file schemas, CLI argument definitions, error messages, tests —
   anything that helps write an accurate and complete man page. Do not
   limit yourself to README and --help; use your full knowledge of the project.
2. Determine the appropriate section: 1 for commands, 5 for config files, 7 for overviews.
3. Write an mdoc(7) source file following the format below.
4. Validate with `mandoc -Tlint <file>` if mandoc is available.
5. If `mansplain` CLI is installed, use `mansplain lint <file>` for additional checks.
6. Place the file at `man/<name>.<section>` in the project.

## Alternative: maintain man pages in markdown

If the `mansplain` CLI is installed, man page source can be maintained in
ronn-format(7) markdown and converted deterministically (no LLM required):

1. Write a `man/toolname.1.md` file in ronn-format markdown
2. Run `mansplain convert man/toolname.1.md -o man/toolname.1`
3. Validate with `mansplain lint man/toolname.1`

This is useful for ongoing human maintenance — the markdown source is
readable and editable without mdoc knowledge. For initial generation,
writing mdoc directly (as this skill teaches) is preferred.

<!-- system-prompt:start -->
Output a complete mdoc(7) man page. No markdown, no explanation, just the mdoc source.

## mdoc(7) format

Every man page starts with this header:

```
.Dd $Mdocdate$
.Dt TOOLNAME 1
.Os
```

- `.Dd` is the document date. Use `$Mdocdate$` or `Month Day, Year`.
- `.Dt` is the document title in UPPERCASE and the section number.
- `.Os` takes no argument (the formatter fills it in).

## Required sections (in order)

```
.Sh NAME
.Nm toolname
.Nd one-line description of what the tool does
```

```
.Sh SYNOPSIS
.Nm
.Op Fl v
.Op Fl o Ar file
.Ar pattern
```

```
.Sh DESCRIPTION
The
.Nm
utility does the thing.
```

## Full example

This is a complete, valid man page. Match this structure exactly.

```
.Dd $Mdocdate$
.Dt GREP 1
.Os
.Sh NAME
.Nm grep
.Nd file pattern searcher
.Sh SYNOPSIS
.Nm
.Op Fl c
.Op Fl i
.Op Fl n
.Op Fl r
.Op Fl v
.Op Fl A Ar num
.Op Fl B Ar num
.Op Fl e Ar pattern
.Op Ar pattern
.Op Ar
.Sh DESCRIPTION
The
.Nm
utility searches input files for lines matching a pattern.
By default, matching lines are printed to standard output.
.Sh OPTIONS
.Bl -tag -width indent
.It Fl c , Fl -count
Print only a count of matching lines.
.It Fl i , Fl -ignore-case
Case insensitive matching.
.It Fl n , Fl -line-number
Prefix each line with its line number.
.It Fl r , Fl -recursive
Recursively search directories.
.It Fl v , Fl -invert-match
Select non-matching lines.
.It Fl A Ar num
Print
.Ar num
lines after each match.
.It Fl B Ar num
Print
.Ar num
lines before each match.
.It Fl e Ar pattern
Specify a search pattern.
.El
.Sh ENVIRONMENT
.Bl -tag -width indent
.It Ev GREP_OPTIONS
Default options prepended to the argument list.
.El
.Sh EXIT STATUS
.Ex -std
.Sh EXAMPLES
Search for a pattern in a file:
.Bd -literal -offset indent
grep 'error' /var/log/syslog
.Ed
.Pp
Recursive case-insensitive search:
.Bd -literal -offset indent
grep -ri 'todo' src/
.Ed
.Sh SEE ALSO
.Xr awk 1 ,
.Xr sed 1
```

## Semantic macros

Use these mdoc macros. Never use raw troff (`.ft`, `.sp`, `.in`, `.br`, `.nf`, `.fi`).

| Macro | Purpose | Example |
|-------|---------|---------|
| `.Nm` | Program name | `.Nm grep` |
| `.Nd` | One-line description | `.Nd file pattern searcher` |
| `.Fl` | Flag | `.Fl v` renders as **-v** |
| `.Ar` | Argument | `.Ar file` renders as _file_ |
| `.Op` | Optional arg | `.Op Fl v` renders as [-v] |
| `.Cm` | Subcommand | `.Cm install` |
| `.Pa` | Path | `.Pa ~/.config/tool` |
| `.Ev` | Environment variable | `.Ev HOME` |
| `.Xr` | Cross-reference | `.Xr grep 1` |
| `.Bl` / `.It` / `.El` | Tagged list | Options list |
| `.Bd` / `.Ed` | Display block | Code examples |
| `.Ex -std` | Standard exit status | Exit status section |

## Section ordering

NAME, SYNOPSIS, DESCRIPTION, OPTIONS, EXIT STATUS, ENVIRONMENT,
FILES, EXAMPLES, SEE ALSO, STANDARDS, HISTORY, AUTHORS, BUGS.

Include at least: NAME, SYNOPSIS, DESCRIPTION, OPTIONS (if flags exist), EXAMPLES.

## Style rules

- Be terse. Man pages are reference material, not tutorials.
- Every flag from the source material gets an `.It` entry in OPTIONS.
- EXAMPLES should have 2-3 realistic invocations a user would actually run.
- Use `.Bd -literal -offset indent` / `.Ed` for code blocks in examples.
- For tools with subcommands, use `.Cm` for subcommand names.
- `.Dt` title must be UPPERCASE.

## Section-specific guidance

For section 1 (user commands): follow the structure and example above exactly.

For section 5 (file formats and config files):
- SYNOPSIS shows the file path, not a command: `.Sh SYNOPSIS` / `.Pa /etc/tool.conf`
- DESCRIPTION documents the file's purpose, syntax, and fields
- No OPTIONS section. Document fields/directives as a tagged list in DESCRIPTION.
- Include EXAMPLES showing realistic file contents in `.Bd -literal` blocks.

For section 7 (overviews, conventions, miscellaneous):
- No SYNOPSIS section.
- DESCRIPTION is the main content — longer prose explaining concepts.
- Use `.Ss` subsection headings to organize topics within DESCRIPTION.
- No OPTIONS section.
- Include at minimum: NAME, DESCRIPTION, SEE ALSO.

<!-- system-prompt:end -->

## Man page sections (not to be confused with mdoc sections above)

| Section | Content | Example |
|---------|---------|---------|
| 1 | User commands | `ls(1)`, `grep(1)` |
| 2 | System calls | `open(2)`, `read(2)` |
| 3 | Library functions | `printf(3)` |
| 5 | File formats | `passwd(5)`, `crontab(5)` |
| 7 | Overviews, conventions | `mdoc(7)`, `regex(7)` |
| 8 | System administration | `mount(8)` |

Most CLI tools go in section 1. Use section 7 for conceptual overviews of
libraries or frameworks. Use section 5 for config file format documentation.

## Validation

After generating, validate the output:

```bash
# If mandoc is available (macOS, OpenBSD, many Linux):
mandoc -Tlint page.1

# If mansplain is installed:
mansplain lint page.1

# Preview the rendered output:
mandoc -Tutf8 page.1 | less
```

## File placement

Place man pages in the project's `man/` directory:

```
project/
  man/
    toolname.1
```

The section number is encoded in the file extension (`.1` for commands,
`.5` for file formats, `.7` for overviews). Package managers and build
tools route files to the correct `man<section>/` directory at install time.
