<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/blurrd-lockup-on-dark.svg">
    <img src="assets/brand/blurrd-lockup-on-light.svg" height="64" alt="blurrd">
  </picture>
</p>

<p align="center">Blur player names in gameplay recordings: fast, on your GPU, fully offline.</p>

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

## Contributing

Contributions are welcome; please read the [contributing guide](CONTRIBUTING.md)
and the [Code of Conduct](CODE_OF_CONDUCT.md) first. Report security problems
as described in the [security policy](SECURITY.md).

## License

blurrd is licensed under the [GNU General Public License 3.0 only](LICENSE). See
[NOTICE](NOTICE) for attribution.

## Legal

blurrd is an independent open-source project and is not affiliated with,
endorsed, sponsored or approved by Riot Games, Inc. VALORANT and Riot Games
are trademarks or registered trademarks of Riot Games, Inc.
