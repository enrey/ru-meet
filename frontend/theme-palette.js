/**
 * Dark theme for the whole palette.
 *
 * Components use raw palette classes (`bg-white`, `text-gray-600`,
 * `bg-blue-50`, ...) rather than semantic tokens, so the dark theme lives here:
 * every color that must change becomes a CSS variable whose light value is the
 * stock Tailwind color (light mode is untouched) and whose `.dark` value is
 * chosen per role. Background, text and border get separate variables because
 * one shade plays different parts: `text-gray-900` must turn light while
 * `bg-gray-900` must stay dark under its `text-white`.
 */
const colors = require('tailwindcss/colors');

const SHADES = ['50', '100', '200', '300', '400', '500', '600', '700', '800', '900', '950'];
const NEUTRALS = ['gray', 'slate', 'zinc', 'neutral', 'stone'];
const HUES = [
  'red', 'orange', 'amber', 'yellow', 'lime', 'green', 'emerald', 'teal', 'cyan',
  'sky', 'blue', 'indigo', 'violet', 'purple', 'fuchsia', 'pink', 'rose',
];

function rgb(hex) {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** `weight` of `a` over `b`, as space-separated channels for `<alpha-value>`. */
function mix(a, b, weight) {
  const [ra, ga, ba] = rgb(a);
  const [rb, gb, bb] = rgb(b);
  const c = (x, y) => Math.round(x * weight + y * (1 - weight));
  return `${c(ra, rb)} ${c(ga, gb)} ${c(ba, bb)}`;
}

const channels = (hex) => rgb(hex).join(' ');

// Dark value of each neutral shade per role, as a shade of the same family.
const NEUTRAL_DARK = {
  bg: { white: '900', 50: ['900', '800', 0.5], 100: '800', 200: '700', 300: '600', 400: '500', 500: '500', 600: '400', 700: '600', 800: '700', 900: '700', 950: '800' },
  text: { 50: '950', 100: '900', 200: '800', 300: '700', 400: '500', 500: '400', 600: '300', 700: '200', 800: '100', 900: '100', 950: '50' },
  border: { white: '900', 50: '800', 100: '800', 200: '800', 300: '700', 400: '600', 500: '500', 600: '400', 700: '300', 800: '200', 900: '100', 950: '50' },
};

function neutralDark(family, spec) {
  const f = colors[family];
  if (Array.isArray(spec)) return mix(f[spec[0]], f[spec[1]], spec[2]);
  return channels(f[spec]);
}

// Hues: pale fills and borders become tints over the dark surface; dark text
// shades become their light counterparts. Saturated shades stay as they are.
const SURFACE = colors.gray['900'];
const HUE_DARK = {
  bg: { 50: 0.12, 100: 0.2, 200: 0.3 },
  border: { 50: 0.2, 100: 0.25, 200: 0.3, 300: 0.45 },
  text: { 600: '400', 700: '300', 800: '200', 900: '100', 950: '50' },
};

function build() {
  const light = {};
  const dark = {};
  const roles = { bg: {}, text: {}, border: {} };

  const define = (role, family, shade, lightHex, darkValue) => {
    const name = `--p-${role}-${family}-${shade}`;
    light[name] = channels(lightHex);
    dark[name] = darkValue;
    roles[role][family] ??= {};
    roles[role][family][shade] = `rgb(var(${name}) / <alpha-value>)`;
  };

  for (const role of ['bg', 'text', 'border']) {
    for (const family of NEUTRALS) {
      for (const shade of SHADES) {
        define(role, family, shade, colors[family][shade], neutralDark(family, NEUTRAL_DARK[role][shade]));
      }
    }
    for (const family of HUES) {
      for (const [shade, spec] of Object.entries(HUE_DARK[role])) {
        const value = typeof spec === 'number'
          ? mix(colors[family]['500'], SURFACE, spec)
          : channels(colors[family][spec]);
        define(role, family, shade, colors[family][shade], value);
      }
    }
  }

  // `white` is a surface for fills and borders; `text-white` stays white.
  for (const role of ['bg', 'border']) {
    const name = `--p-${role}-white`;
    light[name] = '255 255 255';
    dark[name] = neutralDark('gray', NEUTRAL_DARK[role].white);
    roles[role].white = `rgb(var(${name}) / <alpha-value>)`;
  }
  dark['--p-text-black'] = '255 255 255';
  light['--p-text-black'] = '0 0 0';
  roles.text.black = 'rgb(var(--p-text-black) / <alpha-value>)';

  return { light, dark, roles };
}

const { light, dark, roles } = build();

module.exports = {
  /** Spread into `theme.extend`. */
  themeExtend: {
    backgroundColor: roles.bg,
    gradientColorStops: roles.bg,
    textColor: roles.text,
    placeholderColor: roles.text,
    borderColor: roles.border,
    divideColor: roles.border,
    ringColor: roles.border,
    ringOffsetColor: roles.border,
    outlineColor: roles.border,
  },
  /** Plugin that declares the variables. */
  plugin: ({ addBase }) => addBase({ ':root': light, '.dark': dark }),
};
