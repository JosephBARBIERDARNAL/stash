# stash

See, add and remove the AI skills installed on your computer.

stash finds skills for Claude Code, Codex, Gemini CLI, Cursor, GitHub Copilot, OpenCode and `~/.agents`, and checks that each one is valid.

## Install

Download the latest build for your system from [Releases](../../releases).

- **macOS**: unzip and move `stash.app` to Applications. The first time, right-click → Open.
- **Windows**: unzip and run `stash.exe`.
- **Linux**: extract and run `./stash`.

Or build it yourself: `cargo install --git <this repo url>`

## Use

- **Browse**: pick a skill on the left to read its files.
- **Add**: click **+ Add** to copy a folder, fetch a git repository, or start a new skill.
- **Remove**: removed skills go to the Trash. For a linked skill, only the link is removed.
- **Validate**: a green dot means the skill is valid, orange means warnings, red means it won't load.

Plugin and synced skills (🔒) are managed by their app, so stash shows them but never changes them.
