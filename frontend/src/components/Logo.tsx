/** Shield mark for the header: a vault shield with a check. Decorative, so it is hidden from screen readers. */
export function Logo() {
  return (
    <svg className="ns-logo" viewBox="0 0 48 48" aria-hidden="true" focusable="false">
      <defs>
        <linearGradient id="ns-shield" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#00ec97" />
          <stop offset="1" stopColor="#3b6bff" />
        </linearGradient>
      </defs>
      <path d="M24 3 6 9.5v13.2C6 33.6 13.4 41.9 24 45c10.6-3.1 18-11.4 18-22.3V9.5L24 3z" fill="url(#ns-shield)" />
      <path d="M24 7.2 9.6 12.3v10.4c0 8.6 5.7 15.3 14.4 18.1 8.7-2.8 14.4-9.5 14.4-18.1V12.3L24 7.2z" fill="#0a0f1f" />
      <path
        d="m16.5 24.2 5.2 5.2 10-10.6"
        fill="none"
        stroke="#00ec97"
        strokeWidth="3.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
