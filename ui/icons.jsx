import React from "react";

const paths = {
  grid: <>
    <rect x="3" y="3" width="7" height="7" rx="1.8" />
    <rect x="14" y="3" width="7" height="7" rx="1.8" />
    <rect x="3" y="14" width="7" height="7" rx="1.8" />
    <rect x="14" y="14" width="7" height="7" rx="1.8" />
  </>,
  play: <path d="m9 5 11 7-11 7Z" />,
  film: <>
    <rect x="3" y="4" width="18" height="16" rx="3" />
    <path d="M7 4v16M17 4v16M3 9h4m-4 6h4m10-6h4m-4 6h4M10 9l4 3-4 3Z" />
  </>,
  sliders: <>
    <path d="M4 6h5m4 0h7M4 12h9m4 0h3M4 18h2m4 0h10" />
    <circle cx="11" cy="6" r="2" /><circle cx="15" cy="12" r="2" /><circle cx="8" cy="18" r="2" />
  </>,
  chevron: <path d="m9 5 7 7-7 7" />,
  down: <path d="m6 9 6 6 6-6" />,
  check: <path d="m5 12 4 4L19 6" />,
  close: <path d="m6 6 12 12M6 18 18 6" />,
  globe: <>
    <circle cx="12" cy="12" r="9" /><ellipse cx="12" cy="12" rx="4" ry="9" />
    <path d="M3 12h18M5 6.5h14M5 17.5h14" />
  </>,
  refresh: <path d="M20 7v5h-5M4 17v-5h5M6.1 6.1A8 8 0 0 1 20 12M4 12a8 8 0 0 0 13.9 5.9" />,
  download: <path d="M12 3v12m-4-4 4 4 4-4M4 16v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3" />,
  external: <path d="M14 3h7v7m0-7L11 13M10 3H6a3 3 0 0 0-3 3v12a3 3 0 0 0 3 3h12a3 3 0 0 0 3-3v-4" />,
  language: <path d="M3 5h12M9 3v2m3 0c-1 6-4 9-9 11m2-8c1 3 4 6 7 7m1 6 4-11 4 11m-6-4h4" />,
  sun: <>
    <circle cx="12" cy="12" r="4" />
    <path d="M12 1v2m0 18v2M1 12h2m18 0h2M4.2 4.2l1.4 1.4m12.8 12.8 1.4 1.4M4.2 19.8l1.4-1.4M18.4 5.6l1.4-1.4" />
  </>,
  moon: <path d="M20.5 14A9 9 0 0 1 10 3a9 9 0 1 0 10.5 11Z" />,
  folder: <path d="M3 7V5a2 2 0 0 1 2-2h5l3 3h6a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z" />,
  search: <><circle cx="10.8" cy="10.8" r="5.8" /><path d="m15 15 4 4" /></>,
  power: <path d="M12 3v8m-5.2-5a8 8 0 1 0 10.4 0" />,
  trash: <path d="M4 6h16M9 6V4h6v2M6 6l1 14h10l1-14m-8 4v6m4-6v6" />,
  monitor: <>
    <rect x="2" y="3" width="20" height="14" rx="2" />
    <path d="M8 21h8m-4-4v4" />
  </>,
  document: <path d="M5 3h10l4 4v14H5zm10 0v5h4M8 12h8m-8 4h8" />,
};

export function Icon({ name, size = 18 }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.65" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {paths[name]}
    </svg>
  );
}

export function PrismMark({ size = 36, centered = false }) {
  return (
    <svg className="application-mark" width={size} height={size} viewBox={centered ? "0 -2.5 36 36" : "0 0 36 36"} fill="none" aria-hidden="true">
      <path d="M18 3 33 28H3L18 3Z" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
      <path d="m18 3 1 25M3 28l16-10 14 10m-14-10 7-1.75" stroke="currentColor" strokeWidth="1.4" />
    </svg>
  );
}
