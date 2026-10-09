---
name: Ru-Meet Desktop
colors:
  background: '#f8fafc'
  on-background: '#0f172a'
  surface: '#ffffff'
  surface-dim: '#f8fafc'
  surface-container-low: '#f8fafc'
  surface-container: '#f1f5f9'
  surface-container-high: '#e2e8f0'
  on-surface: '#0f172a'
  on-surface-variant: '#64748b'
  outline: '#e2e8f0'
  outline-variant: '#f1f5f9'
  outline-strong: '#cbd5e1'
  primary: '#4f46e5'
  primary-hover: '#4338ca'
  on-primary: '#ffffff'
  primary-container: '#eef2ff'
  on-primary-container: '#4338ca'
  primary-accent: '#6366f1'
  focus-ring: '#e0e7ff'
  focus-border: '#a5b4fc'
  record: '#ef4444'
  record-hover: '#dc2626'
  on-record: '#ffffff'
  text-primary: '#0f172a'
  text-body: '#1e293b'
  text-secondary: '#475569'
  text-muted: '#64748b'
  text-faint: '#94a3b8'
  success: '#047857'
  success-container: '#ecfdf5'
  warning: '#f59e0b'
  on-warning-container: '#b45309'
  warning-container: '#fffbeb'
  error: '#dc2626'
  on-error-container: '#b91c1c'
  error-container: '#fef2f2'
  search-highlight: '#fef08a'
  speaker-1: '#6366f1'
  speaker-2: '#06b6d4'
  speaker-3: '#f59e0b'
  speaker-4: '#ec4899'
  speaker-5: '#10b981'
  speaker-6: '#8b5cf6'
  dark-background: '#0f172a'
  dark-surface: '#0f172a'
  dark-surface-raised: '#1e293b'
  dark-surface-subtle: '#172033'
  dark-outline: '#1e293b'
  dark-input: '#334155'
  dark-on-surface: '#f1f5f9'
  dark-on-surface-variant: '#94a3b8'
  dark-primary-container: '#192142'
  dark-on-primary-container: '#a5b4fc'
typography:
  page-title:
    fontFamily: Source Sans 3
    fontSize: 24px
    fontWeight: '700'
    lineHeight: 32px
    letterSpacing: -0.025em
  settings-title:
    fontFamily: Source Sans 3
    fontSize: 30px
    fontWeight: '700'
    lineHeight: 36px
    letterSpacing: '0'
  onboarding-display:
    fontFamily: Source Sans 3
    fontSize: 36px
    fontWeight: '600'
    lineHeight: 40px
    letterSpacing: '0'
  meeting-title:
    fontFamily: Source Sans 3
    fontSize: 20px
    fontWeight: '600'
    lineHeight: 28px
    letterSpacing: '0'
  section-title:
    fontFamily: Source Sans 3
    fontSize: 20px
    fontWeight: '600'
    lineHeight: 28px
    letterSpacing: '0'
  transcript-body:
    fontFamily: Source Sans 3
    fontSize: 16px
    fontWeight: '400'
    lineHeight: 26px
    letterSpacing: '0'
  body:
    fontFamily: Source Sans 3
    fontSize: 14px
    fontWeight: '400'
    lineHeight: 20px
    letterSpacing: '0'
  body-strong:
    fontFamily: Source Sans 3
    fontSize: 14px
    fontWeight: '600'
    lineHeight: 20px
    letterSpacing: '0'
  caption:
    fontFamily: Source Sans 3
    fontSize: 12px
    fontWeight: '400'
    lineHeight: 16px
    letterSpacing: '0'
  table-header-caps:
    fontFamily: Source Sans 3
    fontSize: 11px
    fontWeight: '700'
    lineHeight: 16px
    letterSpacing: 0.05em
  timestamp-mono:
    fontFamily: ui-monospace
    fontSize: 12px
    fontWeight: '400'
    lineHeight: 16px
    letterSpacing: '0'
rounded:
  sm: 0.25rem
  DEFAULT: 0.375rem
  md: 0.375rem
  lg: 0.5rem
  xl: 0.75rem
  2xl: 1rem
  full: 9999px
spacing:
  unit: 4px
  xs: 4px
  sm: 8px
  md: 12px
  lg: 16px
  xl: 20px
  2xl: 24px
  3xl: 32px
  page-gutter: 32px
  sidebar-collapsed: 64px
  sidebar-expanded: 256px
  nav-item-height: 44px
  table-row-height: 68px
  control-height: 40px
  control-height-compact: 32px
---

# Design System: Ru-Meet Desktop

> Extracted from `frontend/` (Next.js 16 + Tailwind 3 + shadcn/ui "new-york" + Radix + lucide-react),
> packaged as a Tauri desktop app (default window 1100×700). The product is branded **Ru-Meet**;
> UI copy is primarily **Russian** (with an English locale), so layouts must tolerate long Cyrillic labels.

## 1. Visual Theme & Atmosphere

Ru-Meet feels like a calm, focused desktop utility — closer to a native productivity tool (Linear,
Notion, Apple Notes) than a marketing web page. Surfaces are crisp white panels resting on an
almost-white cool-slate canvas, separated by hairline borders rather than shadows. Neutrals are cool
(one slate family throughout), whitespace is moderate and disciplined, and the interface leans
information-dense where it matters: the meeting library is a real data table, the meeting page stacks
a header, an audio player, a long transcript and a speakers column.

Color is used sparingly and with intent. **Indigo** marks "where you are" (active navigation, selected
tab, selected row, focus halos). A single saturated **recording red** is reserved for the act of
recording — the primary CTA of the whole app — and its live indicators. Amber, emerald and red pastel
chips communicate processing state. A full **dark theme** mirrors every surface onto deep slate
(`#0f172a`), turning pale tints into low-opacity color washes over the dark surface.

**Key characteristics**
- Persistent left sidebar (icon rail ↔ labeled), content to the right, no top app bar.
- Flat surfaces, 1px borders, `shadow-sm` only on cards, inputs and floating controls.
- Rounded but not playful: 6–8px for controls, 12px for nav items and table cards, pills for chips.
- Tabular numerals for every time, duration and count.
- Motion is short (150–300ms color/width transitions) and respects `prefers-reduced-motion`.

## 2. Color Palette & Roles

### Primary Foundation
- **Slate Canvas** (`#f8fafc`, slate-50) — app background behind every screen (home, library, settings, onboarding).
- **Paper White** (`#ffffff`) — sidebar, table card, meeting header, transcript and summary panels, dialogs.
- **Cool Wash** (`#f1f5f9`, slate-100) — table header band, empty-state icon tiles, neutral chips.
- **Hairline Slate** (`#e2e8f0`, slate-200; inputs use slate-300 `#cbd5e1`) — all panel borders and dividers; row dividers use the lighter `#f1f5f9`.

### Accent & Interactive
- **Focus Indigo** (`#6366f1`, indigo-500) — active tab underline, selected-row left bar, speaker #1.
- **Deep Indigo** (`#4f46e5`, indigo-600; hover `#4338ca`) — **the single primary button style**: summary generation, dialog "Save", onboarding "Continue", any non-recording primary action.
- **Indigo Mist** (`#eef2ff`, indigo-50) — active sidebar item fill (text `#4338ca`), selected table row (60% opacity), focus ring `#e0e7ff`.
- **Indigo states** — settings tab indicator, switch "on", playing timestamp and transcript highlight all use indigo (500/600); focus = indigo-300 border + indigo-100 ring. *Blue is not part of the system.*
- **Recording Red** (`#ef4444`, red-500; hover `#dc2626`) — "Start recording / New recording" buttons, live recording dot (pulsing). Never used decoratively.

### Typography & Text Hierarchy
- **Slate Ink** (`#0f172a`) — page titles, meeting titles in rows.
- **Graphite** (`#1e293b`, slate-800) — transcript body text.
- **Steel** (`#475569`) — inactive nav labels, secondary body.
- **Fog** (`#64748b`) — metadata, previews, captions, table headers.
- **Faint** (`#94a3b8`) — icons in inputs, placeholders, timestamps at rest, separators "·".

### Functional States
- **Done / Summary ready** — Emerald text `#047857` on `#ecfdf5`, with a 6px dot.
- **Processing / Paused / Recoverable** — Amber text `#b45309` on `#fffbeb`; dot `#f59e0b`; spinner icon.
- **Error / Destructive** — Red text `#b91c1c` on `#fef2f2`; destructive buttons `#dc2626`.
- **Transcript-only (neutral)** — Slate text `#475569` on `#f1f5f9`.
- **Search match** — Butter highlight `#fef08a` behind matched words; "Match:" label in amber `#d97706`.

### Speaker Palette (diarization)
Ordered, cycling: Indigo `#6366f1`, Cyan `#06b6d4`, Amber `#f59e0b`, Pink `#ec4899`, Emerald `#10b981`, Violet `#8b5cf6`.
Used for speaker dots and avatars, the talk-time bar and the vertical speaker timeline.

### Dark Theme Mapping
- Canvas and panels → **Midnight Slate** `#0f172a`; raised popovers/menus → `#1e293b`; subtle fills → `#172033`.
- Borders → `#1e293b`; input borders → `#334155`.
- Text → `#f1f5f9` primary, `#94a3b8` muted.
- Pale tints become washes: indigo-50 → `#192142` with indigo-300 `#a5b4fc` text; emerald/amber/red pastels follow the same 12–20% wash rule.
- Recording red, speaker colors and saturated buttons stay unchanged.

## 3. Typography Rules

**Family:** **Source Sans 3** (variable 400–700, bundled locally with Cyrillic subsets), falling back to
Segoe UI / system-ui. A humanist sans with open apertures — friendly, highly legible in Russian, and
slightly narrower than Inter, which helps long Cyrillic labels fit. Monospace (system `ui-monospace`) is
used only for transcript timestamps.

### Hierarchy & Weights
| Role | Size / Line | Weight | Notes |
|:--|:--|:--|:--|
| Onboarding display | 36 / 40 | 600 | Centered, fade-in-up entrance |
| Settings page title | 30 / 36 | 700 | |
| Library page title | 24 / 32 | 700 | `tracking-tight`, followed by a count pill |
| Meeting title | 20 / 28 | 600 | Up to 2 lines, inline-editable on click |
| Settings section title | 20 / 28 | 600 | |
| Transcript text | 16 / 26 | 400 | `leading-relaxed`, Graphite |
| Body / nav label / button | 14 / 20 | 500–600 | Buttons and tabs are semibold |
| Caption / metadata | 12 / 16 | 400–500 | Previews, date second line, chips |
| Table header | 11 / 16 | 700 | UPPERCASE, `tracking-wider`, Fog |
| Micro badge | 10–11 | 500–700 | Kbd hints ("Ctrl K"), counters |

### Spacing Principles
- Numbers always use `tabular-nums` (times, durations, counts) so columns don't jitter.
- Headings use tight or default tracking; only uppercase table headers are letter-spaced.
- Two-line list cells pair a 14px semibold title with a 12px muted caption, 2px apart.

## 4. Component Stylings

### Buttons
- **Record CTA (primary of the app):** Recording Red fill, white 14px semibold label with a mic icon, `rounded-lg` (8px), `px-4 py-2`, `shadow-sm`, hover darkens to `#dc2626`, focus ring red-300. In the sidebar it is a full-width 44px nav item (`rounded-xl`), becoming a red circle in the collapsed rail; while recording it shows a filled square "Stop".
- **Secondary / outline:** white fill, 1px slate-200 border, slate-700 semibold text, `shadow-sm`, hover to slate-50.
- **Primary (non-recording):** Deep Indigo fill, white 14px semibold text, `rounded-md` (6px), `shadow-sm`, hover `#4338ca`; heights sm 32px, default 36px, lg 40–44px, icon 36×36. Used for summary generation, dialog Save, onboarding Continue, settings Save. There is no black or blue primary.
- **Ghost / icon:** transparent, 28–32px square, `rounded-md`, slate-500 icon, hover slate-100 fill. Row actions (rename/delete) are 32px bordered white squares that reveal on row hover; delete hover tints red.
- **Destructive soft:** red-50 fill, red-200 border, red-700 text.
- Disabled = 50–60% opacity. Transitions are color-only, 150ms.

### Cards & Containers
- **Table card:** white, 1px slate-200 border, `rounded-xl` (12px), `shadow-sm`, fills remaining height with internal scroll.
- **Onboarding card:** white, `rounded-lg`, slate-200 border, `shadow-sm`, 24px padding, max-width 448px.
- **Dialogs:** shadcn dialog, white, rounded, centered; widths 425px (rename) to 560px (errors). Footer buttons right-aligned: outline/slate-100 "Cancel" + Deep Indigo "Save".
- **Floating audio-status widget:** 320px wide, `rounded-md`, `shadow-lg`, draggable over the home transcript.
- **Status overlays:** centered spinner + message over content while processing/saving.

### Navigation
- **Sidebar:** fixed left, full height, white with right hairline border, `py-4`. Collapsed = 64px icon rail (44×44 icon buttons, tooltips on the right); expanded = 256px with `px-3`, 20px lucide icons + 14px medium labels, optional second-line detail (e.g. "Идёт запись · 12:04") and trailing count.
- Order: Logo (40px app icon + "Ru-Meet" + version) → Home → Meetings (count) → Import audio (beta) → **red Record button** → Interrupted meetings (amber, with count pill) → spacer → Settings → Theme toggle → Collapse.
- **Active item:** Indigo Mist fill, indigo-700 text, `rounded-xl`. Inactive: slate-600, hover slate-100.
- **Live badge:** 10px red pulsing dot (amber when paused) with a 2px white ring on the Home icon corner.
- Width animates over 300ms; main content shifts margin in sync.
- **Underline tabs (meeting page):** 14px semibold labels, 24px gap, 2px indigo-500 bottom border on active; inactive slate-500.
- **Underline tabs (settings):** icon + label, indigo-600 active text with an animated 2px indigo-500 sliding indicator (same accent as meeting tabs).
- **Segmented control (filters):** slate-200/70 track, `rounded-lg`, 4px padding; active segment white with `shadow-sm`, slate-900 text.

### Inputs & Forms
- **Search field:** 40px tall, max-width 448px, white, slate-200 border, `rounded-lg`, `shadow-sm`, leading search icon (spinner while searching), trailing "Ctrl K" kbd chip or clear ×. Focus: indigo-300 border + 2px indigo-100 ring.
- **Compact search (in-meeting):** 32px tall, 288px wide, `rounded-md`, result counter "3/12" and up/down 28px icon buttons.
- **Text input (dialogs):** slate-300 border, `rounded-md`, `px-3 py-2`; focus indigo-300 border + 2px indigo-100 ring (same as search).
- **Switch:** shadcn switch, checked = indigo-600.
- **Selects/popovers/menus:** shadcn Radix defaults, 6px radius, white (dark: `#1e293b`).

### Meeting Library Table
Columns: Title (flex) · Date 130px · Duration 120px · Speakers 90px · Status 130px · Actions 76px, 16px column gap, 20px side padding.
- Sticky header band 40px, slate-100/80, uppercase 11px bold labels.
- Date-group headers ("Сегодня", "Вчера", …) are sticky 12px semibold strips on translucent slate-50 with backdrop blur and a count pill.
- Rows 68px: 36px rounded-lg tinted icon tile (indigo = summary ready, amber = processing, red = failed, slate = transcript only), title + preview/match line, time over short date, duration, speakers (users icon + n), status chip, hover-revealed actions.
- Selected row: indigo-50/60 fill + 4px indigo-500 bar on the left edge.
- Empty state: 56px rounded-2xl slate tile with audio-lines icon, semibold headline, muted helper text.

### Status Chips
Pill (`rounded-full`), `px-2.5 py-0.5`, 12px medium text, pastel fill per state (see Functional States). Ready chips start with a 6px dot in currentColor; processing chips start with a 12px spinner.

### Meeting Page (header + player + transcript/summary + speakers column)
- **Header:** one white row, `px-8 py-3`, slate-100 bottom border: "← Meetings" ghost link, 4px slate-300 dot, editable 18px semibold title (truncates, pencil on hover), status chip; date · duration · speakers in 12px slate-400 on the right from 64rem; then actions. Visible actions are only Folder (ghost), Save (only while the summary has unsaved edits), the indigo summary button and "⋯"; diarization and reading the summary aloud start from "⋯" and show a visible stop control while they run.
- **Player:** slim bar under the header, shown when the recording is playable: ±10 s buttons, 32px indigo round play button, mono `current / total`, a 3px scrubber (knob on hover/drag), speed segmented control (1x–2x) in a slate-100 pill, volume from 56rem. Space toggles playback outside inputs. Once played, the recording keeps playing when the page is left: the same controls dock under the recording activity bar on every other page, with the meeting title (opens the meeting) and a small slate-400 close cross in the top-right corner that stops and unloads it.
- **Tabs row:** segmented tabs (Transcript first, with a mono count badge; active = slate-100 fill + indigo-700 text), search field on the right (slate-100 fill, no border until focused).
- **Transcript:** consecutive lines of one speaker form a turn: 28px round avatar (speaker number or initials on a 15% speaker-color tint), speaker name + mono time, 15px relaxed text. Hovered turn gets a slate-50 fill and play/copy icons. The playing turn is tinted indigo-50/70 with a solid avatar and a small pulsing wave; the playing line inside it gets indigo-100. Clicking a line plays from it.
- **Speakers column (transcript tab only, 300px, from 52rem):** borderless on white with a slate-100 left border. Title + count, a 4px stacked talk-time bar, then a 4px vertical timeline of all turns (top = start) with an indigo playback dot and a hover tooltip, next to the speaker list (color dot, name, `minutes · share` in slate-400). Clicking a speaker moves to their next turn; the pencil renames.
- **Speaker name:** optional 10px color dot + label; click opens a 288px popover to rename/merge.
- **Summary panel:** BlockNote rich-text editor, full-width, `px-8 py-6`.

### Recording Activity Bar & Home
- Activity bar: white top strip with hairline bottom border, `px-8 py-2`, holding one full-width 40px `rounded-lg` tinted row (`pl-2.5 pr-3`) so the state and its control read as one unit — indigo-50 with animated ((•)) echo icon while auto-record listens, red-50 with a pulsing 10px dot + tabular timer while recording, amber-50 when paused, slate-100 when off. Left: semibold 14px label + 12px slate-500 description. Right, inside the same tint: 12px "Вкл/Выкл" + 36×20 indigo switch, or ghost pause icon + white red-outline 28px "Стоп" button while recording.
- Home = live transcript column, centered, `w-2/3 max-w-[750px]`, on Slate Canvas, with the draggable audio-status card.

### Onboarding
Full-screen slate-50 overlay, centered stack: step indicator with arrows → 36px title + 16px slate-600 subtitle → white checklist card → full-width 44px Deep Indigo primary button. Language switcher is a rounded-full segmented pill (active = indigo-600 fill, white text).

## 5. Layout Principles

### Grid & Structure
- Desktop shell: `[Sidebar 64|256px fixed] [Main: flex column, h-screen, overflow hidden]`.
- Main content has a 32px left gutter (`pl-8`); library and meeting header use 32px side padding.
- Settings content is centered with `max-w-6xl` (1152px); home transcript `max-w-[750px]`.
- Screens never scroll as a whole: headers are fixed, inner panels scroll (thin 6px custom scrollbar, slate-300 thumb).
- Container queries (`@container`) drive compaction of meeting toolbars.

### Whitespace Strategy
- 4px base unit; common steps 8 / 12 / 16 / 20 / 24 / 32px.
- Page vertical padding ~28px on the library; 20px between header and toolbar, 16px between toolbar and table.
- Settings sections: 32px vertical padding separated by hairline borders.

### Alignment & Visual Balance
- Left-aligned everything in work screens; centered only in onboarding, empty states and overlays.
- Header rows: title left, actions pushed right (`ml-auto`), 8px gaps.
- Visual weight concentrates on one red CTA per screen; everything else stays neutral.

### Responsive Behavior & Touch
- Desktop-first, minimum practical width ~900px; the sidebar collapses to a rail to give space.
- Buttons/hit targets: 32px compact, 36–44px standard; nav items 44px.
- Long Russian labels truncate with ellipsis; action-button labels hide below container breakpoints, leaving icons + tooltips.
- Keyboard: visible focus rings everywhere (indigo), Ctrl+K focuses library search, Enter/Space opens rows.

## 6. Design System Notes for Stitch Generation

### Language to Use
"Clean, calm desktop productivity app for recording and transcribing meetings; cool off-white canvas,
white panels with hairline slate borders, minimal shadows, indigo for selection and focus, a single
vivid red recording button, pastel status pills, Source Sans 3, Russian UI text, persistent left sidebar."
Generate as **desktop** screens (1100–1440px wide), light theme first, then a dark variant.

### Color References
- Canvas: Slate Canvas `#f8fafc`; panels Paper White `#ffffff`; borders Hairline Slate `#e2e8f0`.
- Selection: Focus Indigo `#6366f1`, Indigo Mist `#eef2ff`, Deep Indigo `#4f46e5`.
- Record: Recording Red `#ef4444`.
- Text: Slate Ink `#0f172a`, Fog `#64748b`, Faint `#94a3b8`.
- States: Emerald `#047857`/`#ecfdf5`, Amber `#b45309`/`#fffbeb`, Red `#b91c1c`/`#fef2f2`.
- Speakers: `#6366f1`, `#06b6d4`, `#f59e0b`, `#ec4899`, `#10b981`, `#8b5cf6`.
- Dark: `#0f172a` surfaces, `#1e293b` raised, `#f1f5f9` text.

### Screens to Recreate (priority order)
1. **Meeting Library** (`/meetings`) — title + count, Import / New recording buttons, search + segmented filter, grouped data table.
2. **Meeting Details** (`/meeting-details`) — one-row header with editable title, metadata and actions, slim audio player, tabs (Transcript / Summary), transcript as speaker turns, right speakers column with a vertical timeline.
3. **Home / Live Recording** (`/`) — activity bar, centered live transcript, floating audio-status card.
4. **Settings** (`/settings`) — big title, underline tabs with icons, stacked form sections.
5. **Onboarding** — welcome, permissions, model download steps.
6. **Sidebar** in collapsed and expanded states (shared shell for all screens).

### Component Prompts
- *Sidebar:* "A 256px white left sidebar with a thin right border. Top: 40px rounded app icon, 'Ru-Meet' in 18px semibold and a grey version number. Nav items 44px tall, 12px rounded corners, 20px line icons and 14px medium labels: 'Главная' (active: very light indigo background, indigo text), 'Встречи' with a grey count '24' on the right. Below them, a full-width solid red (#ef4444) button 'Начать запись' with a microphone icon. At the bottom: Settings, theme toggle, collapse."
- *Library table:* "A white card with 12px corners, slate-200 border and a soft small shadow. Header band in pale slate with uppercase 11px bold grey labels: Название, Дата, Длительность, Спикеры, Статус. Rows 68px tall separated by very light lines; each starts with a 36px rounded indigo-tinted square containing a waveform icon, a bold 14px meeting title and a grey 12px preview line. Status shown as small pastel pills: green 'Готово' with a dot, amber 'Обработка' with spinner. Sticky date group headers 'Сегодня', 'Вчера' with count pills. Selected row has a light indigo fill and a 4px indigo bar on the left edge."
- *Speakers column:* "A light 300px right column on white: a 4px rounded bar split into speaker colors by talk time, then a 4px vertical line of thin colored segments (one per speech turn, indigo dot at the playback position) beside a borderless list of speakers — color dot, name, and muted `28 min · 36%`."
- *Transcript:* "Transcript lines with a grey monospace timestamp column (00:12:34) on the left, a small colored speaker dot and bold speaker name, and 16px relaxed dark-grey text. One line highlighted with a pale indigo fill and indigo ring."

### Incremental Iteration
1. Lock the shell first (sidebar + canvas + one-red-CTA rule), then generate each screen inside it.
2. Keep borders over shadows; if Stitch adds heavy drop shadows or gradients, ask to "flatten to hairline borders and shadow-sm only".
3. Keep indigo for selection and red only for recording; reject extra brand colors.
4. Verify Cyrillic text fits — ask for truncation with ellipsis rather than wrapping in table cells and nav.
5. After light screens are approved, request the dark theme: "convert to dark mode with #0f172a surfaces, #1e293b popovers, #f1f5f9 text, pastel fills as 15% color washes, red and speaker colors unchanged".

### Unification Decisions
This document describes the **target** system, which unifies three inconsistencies found in the code:
- **Accent:** indigo only. Blue-600 (settings tabs, dialog Save, switches, transcript playback/highlight, rename inputs) is replaced by indigo.
- **Neutrals:** slate only. Gray-* surfaces, text and borders (home, settings, onboarding, meeting page) map to the same slate shade; dark theme follows slate-900/800.
- **Primary button:** one Deep Indigo style. The shadcn near-black default, the black onboarding button and blue dialog buttons are retired. Recording Red remains the only other filled CTA.
