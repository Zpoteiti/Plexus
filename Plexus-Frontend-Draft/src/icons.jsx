// Minimal stroked icons, 16px, currentColor
const Icon = ({ d, size = 16, fill = false, children }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="none"
       stroke={fill ? "none" : "currentColor"} strokeWidth="1.4"
       strokeLinecap="round" strokeLinejoin="round" className="nav-icon">
    {children || <path d={d} />}
  </svg>
);

const Icons = {
  Chat: () => <Icon><path d="M3 4.5C3 3.67 3.67 3 4.5 3h7c.83 0 1.5.67 1.5 1.5v5c0 .83-.67 1.5-1.5 1.5H7.5L5 13.5V11h-.5C3.67 11 3 10.33 3 9.5v-5z"/></Icon>,
  Device: () => <Icon><rect x="2" y="3.5" width="12" height="8" rx="1"/><path d="M5.5 13.5h5M8 11.5v2"/></Icon>,
  Workspace: () => <Icon><path d="M2.5 4.5C2.5 3.94 2.94 3.5 3.5 3.5h2.59c.27 0 .52.1.71.29l1.41 1.42c.19.19.44.29.71.29H12.5c.55 0 1 .45 1 1V11.5c0 .55-.45 1-1 1h-9c-.55 0-1-.45-1-1V4.5z"/></Icon>,
  Cron: () => <Icon><circle cx="8" cy="8" r="5.5"/><path d="M8 5v3l2 1.5"/></Icon>,
  Channel: () => <Icon><path d="M2.5 8a5.5 5.5 0 1 0 11 0 5.5 5.5 0 1 0-11 0M2.5 8h11M8 2.5c1.5 1.6 2.4 3.6 2.4 5.5s-.9 3.9-2.4 5.5M8 2.5c-1.5 1.6-2.4 3.6-2.4 5.5s.9 3.9 2.4 5.5"/></Icon>,
  Admin: () => <Icon><path d="M8 2.5l5 2v3.5c0 3-2 5-5 5.5-3-.5-5-2.5-5-5.5V4.5l5-2z"/></Icon>,
  Plus: () => <Icon><path d="M8 3v10M3 8h10"/></Icon>,
  Search: () => <Icon><circle cx="7" cy="7" r="4"/><path d="m13 13-3-3"/></Icon>,
  Send: () => <Icon><path d="M2.5 8L13.5 3l-2.5 10.5L8 9.5 2.5 8z"/></Icon>,
  Stop: () => <Icon><rect x="4" y="4" width="8" height="8" rx="1"/></Icon>,
  Sun: () => <Icon><circle cx="8" cy="8" r="3"/><path d="M8 1.5v1.5M8 13v1.5M3 3l1 1M12 12l1 1M1.5 8H3M13 8h1.5M3 13l1-1M12 4l1-1"/></Icon>,
  Moon: () => <Icon><path d="M13 9.5A5.5 5.5 0 0 1 6.5 3a5.5 5.5 0 1 0 6.5 6.5z"/></Icon>,
  Sliders: () => <Icon><path d="M3 4h6M11 4h2M3 8h2M7 8h6M3 12h8M13 12h0"/><circle cx="10" cy="4" r="1.5"/><circle cx="6" cy="8" r="1.5"/><circle cx="12" cy="12" r="1.5"/></Icon>,
  ChevronDown: () => <Icon><path d="m4 6 4 4 4-4"/></Icon>,
  ChevronRight: () => <Icon><path d="m6 4 4 4-4 4"/></Icon>,
  More: () => <Icon><circle cx="3.5" cy="8" r="1" fill="currentColor" stroke="none"/><circle cx="8" cy="8" r="1" fill="currentColor" stroke="none"/><circle cx="12.5" cy="8" r="1" fill="currentColor" stroke="none"/></Icon>,
  Folder: () => <Icon><path d="M2.5 4.5c0-.55.45-1 1-1h2.6c.27 0 .52.1.71.29l1.4 1.42c.2.19.45.29.71.29H12.5c.55 0 1 .45 1 1V11.5c0 .55-.45 1-1 1h-9c-.55 0-1-.45-1-1V4.5z"/></Icon>,
  File: () => <Icon><path d="M3.5 2h5.5L13 6v7.5c0 .28-.22.5-.5.5h-9c-.28 0-.5-.22-.5-.5V2.5c0-.28.22-.5.5-.5z"/><path d="M9 2v4h4"/></Icon>,
  Tool: () => <Icon><path d="M10.5 5.5l3 3-4 4-3-3 4-4z"/><path d="M5.5 6.5L3 4l1.5-1.5L7 5l-1.5 1.5z"/><path d="m6 6 4 4"/></Icon>,
  Image: () => <Icon><rect x="2" y="3" width="12" height="10" rx="1"/><circle cx="6" cy="7" r="1.2"/><path d="m2.5 11 3-3 4 4 1.5-1.5 2.5 2.5"/></Icon>,
  Discord: () => <Icon><path d="M3 11c1.5 1 3.5 1.5 5 1.5s3.5-.5 5-1.5M5.5 9.5c.5.5 1 .5 1.5 0M9 9.5c.5.5 1 .5 1.5 0M4 5l1-1.5h6L12 5l1 6c-1.5 1-3.5 1.5-5 1.5S4.5 12 3 11l1-6z"/></Icon>,
  Telegram: () => <Icon><path d="m2.5 8 11-4.5-2 10-3.5-2-1.5 2-1-3 7-5L4.5 8.5 2.5 8z"/></Icon>,
  Web: () => <Icon><circle cx="8" cy="8" r="5.5"/><path d="M2.5 8h11M8 2.5c1.5 1.6 2.4 3.6 2.4 5.5s-.9 3.9-2.4 5.5M8 2.5c-1.5 1.6-2.4 3.6-2.4 5.5s.9 3.9 2.4 5.5"/></Icon>,
  Lock: () => <Icon><rect x="3.5" y="7" width="9" height="6" rx="1"/><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2"/></Icon>,
  Copy: () => <Icon><rect x="5" y="5" width="8" height="8" rx="1"/><path d="M3 11V4c0-.55.45-1 1-1h7"/></Icon>,
  Refresh: () => <Icon><path d="M3 8a5 5 0 0 1 9-3l1.5 1.5M13 4v3h-3M13 8a5 5 0 0 1-9 3L2.5 9.5M3 12V9h3"/></Icon>,
  Check: () => <Icon><path d="M3 8.5 6.5 12 13 4.5"/></Icon>,
  X: () => <Icon><path d="M3.5 3.5l9 9M12.5 3.5l-9 9"/></Icon>,
  Power: () => <Icon><path d="M5.5 4.5a5 5 0 1 0 5 0M8 2v6"/></Icon>,
  Sparkle: () => <Icon><path d="M8 2v3M8 11v3M2 8h3M11 8h3M4 4l2 2M10 10l2 2M4 12l2-2M10 6l2-2"/></Icon>,
  Compaction: () => <Icon><path d="M2 5h12M2 8h8M2 11h12"/><path d="M11 7l2 1-2 1z" fill="currentColor"/></Icon>,
  Trash: () => <Icon><path d="M3 4h10M5.5 4V2.5h5V4M4 4l.5 9h7L12 4M6.5 6.5v5M9.5 6.5v5"/></Icon>,
  Settings: () => <Icon><circle cx="8" cy="8" r="2"/><path d="m13 8.5-1-.5.4-1.2-1-1-1.2.4-.5-1H8l-.5 1-1.2-.4-1 1 .4 1.2-1 .5v1l1 .5-.4 1.2 1 1 1.2-.4.5 1h1l.5-1 1.2.4 1-1-.4-1.2 1-.5z"/></Icon>,
  Bell: () => <Icon><path d="M4 11V8a4 4 0 0 1 8 0v3l1 1.5H3L4 11zM6.5 14a1.5 1.5 0 0 0 3 0"/></Icon>,
  Logout: () => <Icon><path d="M9 11v1.5c0 .28-.22.5-.5.5h-5c-.28 0-.5-.22-.5-.5v-9c0-.28.22-.5.5-.5h5c.28 0 .5.22.5.5V5"/><path d="M6 8h7M11 5.5 13.5 8 11 10.5"/></Icon>,
  Sphere: () => <Icon><circle cx="8" cy="8" r="5.5"/><ellipse cx="8" cy="8" rx="2.5" ry="5.5"/><path d="M2.5 8h11"/></Icon>,
  Server: () => <Icon><rect x="2.5" y="3" width="11" height="4" rx="1"/><rect x="2.5" y="9" width="11" height="4" rx="1"/><circle cx="5" cy="5" r="0.5" fill="currentColor"/><circle cx="5" cy="11" r="0.5" fill="currentColor"/></Icon>,
};

window.Icon = Icon;
window.Icons = Icons;
