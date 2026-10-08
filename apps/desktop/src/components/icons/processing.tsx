import { createIcon } from "./icon";

/** Gaussian blur style: a sharp core fading out in soft rings. */
export const BlurStyleIcon = createIcon(
  "BlurStyleIcon",
  <>
    <circle cx="12" cy="12" r="3" />
    <circle cx="12" cy="12" r="6" strokeOpacity="0.6" strokeDasharray="2.5 2" />
    <circle cx="12" cy="12" r="9" strokeOpacity="0.3" strokeDasharray="1.5 3" />
  </>,
);

/** Pixelate style: a 2 x 2 block grid with alternating intensity. */
export const PixelateIcon = createIcon(
  "PixelateIcon",
  <>
    <rect x="4" y="4" width="7" height="7" rx="1" />
    <rect x="13" y="4" width="7" height="7" rx="1" strokeOpacity="0.45" />
    <rect x="4" y="13" width="7" height="7" rx="1" strokeOpacity="0.45" />
    <rect x="13" y="13" width="7" height="7" rx="1" />
  </>,
);

/** OCR verification: a scan frame around a check mark. */
export const OcrVerifyIcon = createIcon(
  "OcrVerifyIcon",
  <>
    <path d="M4 8V5.5A1.5 1.5 0 0 1 5.5 4H8" />
    <path d="M16 4h2.5A1.5 1.5 0 0 1 20 5.5V8" />
    <path d="M20 16v2.5a1.5 1.5 0 0 1-1.5 1.5H16" />
    <path d="M8 20H5.5A1.5 1.5 0 0 1 4 18.5V16" />
    <path d="m8.5 12.25 2.5 2.5 4.5-5" />
  </>,
);

/** GPU and hardware encoder (NVENC, AMF, QSV): a graphics card with a fan. */
export const GpuIcon = createIcon(
  "GpuIcon",
  <>
    <rect x="3" y="6" width="18" height="11" rx="2" />
    <circle cx="9" cy="11.5" r="2.75" />
    <path d="M15 10h3" />
    <path d="M15 13h3" />
    <path d="M6.5 17v2" />
    <path d="M10 17v2" />
    <path d="M13.5 17v2" />
  </>,
);
