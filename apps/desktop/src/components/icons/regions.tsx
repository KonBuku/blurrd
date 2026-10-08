import { createIcon } from "./icon";

/** Kill feed: stacked feed rows next to a crosshair. */
export const KillFeedIcon = createIcon(
  "KillFeedIcon",
  <>
    <circle cx="7" cy="12" r="3.5" />
    <path d="M7 5.5v2" />
    <path d="M7 16.5v2" />
    <path d="M14 6h7" />
    <path d="M14 12h7" />
    <path d="M14 18h7" />
  </>,
);

/** Round banner (ACE, clutch, round result): a ribbon banner. */
export const RoundBannerIcon = createIcon(
  "RoundBannerIcon",
  <>
    <rect x="4" y="5" width="16" height="9" rx="2" />
    <path d="M8 14v5l2-1.5 2 1.5v-5" />
    <path d="M8.5 9.5h7" />
  </>,
);

/** Voice chat speaker list: a microphone. */
export const VoiceChatIcon = createIcon(
  "VoiceChatIcon",
  <>
    <rect x="9" y="3" width="6" height="11" rx="3" />
    <path d="M5.5 11a6.5 6.5 0 0 0 13 0" />
    <path d="M12 17.5V21" />
  </>,
);

/** Text chat: a speech bubble with message lines. */
export const TextChatIcon = createIcon(
  "TextChatIcon",
  <>
    <path d="M5.5 4h13A1.5 1.5 0 0 1 20 5.5v9a1.5 1.5 0 0 1-1.5 1.5H10l-4.5 4v-4A1.5 1.5 0 0 1 4 14.5v-9A1.5 1.5 0 0 1 5.5 4Z" />
    <path d="M8 8.5h8" />
    <path d="M8 11.5h5" />
  </>,
);

/** Scoreboard / tab screen: a table with a header row. */
export const ScoreboardIcon = createIcon(
  "ScoreboardIcon",
  <>
    <rect x="3" y="4" width="18" height="16" rx="2" />
    <path d="M3 9h18" />
    <path d="M3 14.5h18" />
    <path d="M9 9v11" />
  </>,
);

/** Death recap / "killed by" panel: a skull. */
export const DeathRecapIcon = createIcon(
  "DeathRecapIcon",
  <>
    <path d="M12 3a7 7 0 0 0-7 7c0 2.4 1.2 4.2 3 5.3V19h8v-3.7c1.8-1.1 3-2.9 3-5.3a7 7 0 0 0-7-7Z" />
    <circle cx="9.5" cy="10.5" r="1.25" />
    <circle cx="14.5" cy="10.5" r="1.25" />
    <path d="M10.5 19v2" />
    <path d="M13.5 19v2" />
  </>,
);

/** "Spectating <name>" banner: an eye. */
export const SpectatingIcon = createIcon(
  "SpectatingIcon",
  <>
    <path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12Z" />
    <circle cx="12" cy="12" r="3" />
  </>,
);

/** Match end, MVP and round end screens: a trophy. */
export const MatchEndIcon = createIcon(
  "MatchEndIcon",
  <>
    <path d="M8 4h8v5a4 4 0 0 1-8 0V4Z" />
    <path d="M8 6H5.5a2.5 2.5 0 0 0 2.6 4" />
    <path d="M16 6h2.5a2.5 2.5 0 0 1-2.6 4" />
    <path d="M12 13v4" />
    <path d="M8.5 20h7" />
    <path d="M10 17h4v3h-4z" />
  </>,
);

/** Manual box drawn by the user: a dashed frame with a pointer. */
export const ManualBoxIcon = createIcon(
  "ManualBoxIcon",
  <>
    <path d="M14 4H6a2 2 0 0 0-2 2v8" strokeDasharray="2.5 2.5" />
    <path d="M20 10V6a2 2 0 0 0-2-2" />
    <path d="M10 20H6a2 2 0 0 1-2-2" />
    <path d="m13 13 7.5 2.5-3.3 1.4-1.4 3.3L13 13Z" />
  </>,
);
