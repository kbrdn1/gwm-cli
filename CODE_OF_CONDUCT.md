# Code of Conduct

## Our pledge

We pledge to make participation in `gwm` a harassment-free experience for everyone, regardless of age, body size, visible or invisible disability, ethnicity, sex characteristics, gender identity and expression, level of experience, education, socio-economic status, nationality, personal appearance, race, religion, or sexual identity and orientation.

We pledge to act and interact in ways that contribute to an open, welcoming, diverse, inclusive, and healthy community.

## Our standards

Examples of behaviour that contributes to a positive environment:

- Using welcoming and inclusive language.
- Being respectful of differing viewpoints and experiences.
- Gracefully accepting constructive criticism.
- Focusing on what is best for the community.
- Showing empathy towards other community members.

Examples of unacceptable behaviour:

- The use of sexualised language or imagery, and unwelcome sexual attention or advances.
- Trolling, insulting or derogatory comments, personal or political attacks.
- Public or private harassment.
- Publishing others' private information (such as a physical or email address) without their explicit permission.
- Other conduct which could reasonably be considered inappropriate in a professional setting.

## Enforcement responsibilities

Project maintainers are responsible for clarifying the standards of acceptable behaviour and are expected to take appropriate and fair corrective action in response to any behaviour they deem inappropriate, threatening, offensive, or harmful.

Maintainers have the right and responsibility to remove, edit, or reject comments, commits, code, wiki edits, issues, and other contributions that are not aligned to this Code of Conduct, and will communicate reasons for moderation decisions when appropriate.

## Scope

This Code of Conduct applies within all project spaces (issues, pull requests, discussions, code review) and also applies when an individual is officially representing the project in public spaces.

## Engineering conduct

Beyond the interpersonal standards above, contributions are also expected to meet a baseline of engineering discipline. The non-negotiable rule is:

- **Test-Driven Development is mandatory.** No production code is merged without a failing test that pinned the behaviour down first. Submitting code that adds or changes behaviour without an accompanying test in `tests/` is considered a breach of contribution conduct, not a stylistic preference. Reviewers will close such PRs with a pointer to [`CONTRIBUTING.md`](CONTRIBUTING.md#-tdd-is-mandatory--non-negotiable) and to the project-level [`CLAUDE.md`](CLAUDE.md) which both spell out the red → green → refactor loop.

This rule exists to protect users (regressions are caught at the boundary, not after release) and reviewers (a PR's test diff is what reviewers read first). It applies equally to human contributors and to changes authored or assisted by AI tools.

Exceptions are narrowly enumerated in `CONTRIBUTING.md` and must be argued explicitly in the PR description.

## Enforcement

Instances of abusive, harassing, or otherwise unacceptable behaviour may be reported to the project maintainer at the contact listed on the GitHub profile. All complaints will be reviewed and investigated promptly and fairly.

All project maintainers are obligated to respect the privacy and security of the reporter of any incident.

## Attribution

This Code of Conduct is adapted from the [Contributor Covenant][homepage], version 2.1, available at <https://www.contributor-covenant.org/version/2/1/code_of_conduct.html>.

[homepage]: https://www.contributor-covenant.org
