# blurrd

Blur player names in gameplay recordings: fast, on your GPU, fully offline.

> **Status: in development.** There is no release yet. Nothing in this
> repository is ready for use, and file formats, the CLI and the user
> interface will change without notice until v0.1.0.

## What it will do

blurrd is a Windows desktop app (plus a command-line tool) that takes a
gameplay recording, finds the player names in the regions you pick (such as
the kill feed, round banners, voice and text chat, the scoreboard or the
death recap), covers them with tight, soft-edged blur boxes that follow every
animation, checks the result with OCR and exports a video with the same
resolution, frame rate, timestamps and audio as the source.

The first supported game is VALORANT (recordings at 1080p and similar
16:9 resolutions).

## License

blurrd is licensed under the [GNU General Public License 3.0 only](LICENSE). See
[NOTICE](NOTICE) for attribution.

## Legal

blurrd is an independent open-source project and is not affiliated with,
endorsed, sponsored or approved by Riot Games, Inc. VALORANT and Riot Games
are trademarks or registered trademarks of Riot Games, Inc.
