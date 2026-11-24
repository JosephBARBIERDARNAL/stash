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

- **Browse**: pick a skill on the left. Pills show which tools have it; click one to see that copy.
- **Add**: click **+ Add** to copy a folder, fetch a git repository, or start a new skill.
- **Remove**: removed skills go to the Trash. For a linked skill, only the link is removed.
- **Validate**: ● valid, ▲ warnings, ■ won't load.

Plugin and synced skills (🔒) are managed by their app, so stash shows them but never changes them.

## Accessibility

- Works with screen readers (VoiceOver, Narrator, Orca).
- Fully usable from the keyboard: Tab to move, ↑ ↓ to pick a skill, ⌘F search, ⌘N add, ⌘R rescan.
- Zoom with ⌘ + / ⌘ − (Ctrl on Windows and Linux).
- Set in [Atkinson Hyperlegible](https://www.brailleinstitute.org/freefont/), with high-contrast colors in light and dark mode.
