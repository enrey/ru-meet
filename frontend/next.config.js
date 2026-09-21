const path = require('path');
const tiptapPmResolveBase = path.dirname(require.resolve('@tiptap/pm/model'));
const resolveFromTiptapPm = (pkg) =>
  require.resolve(pkg, { paths: [tiptapPmResolveBase] });

// BlockNote and Tiptap both pull in ProseMirror. Two copies of any
// prosemirror-* package in one bundle break the editor at runtime (duplicate
// prototypes, "Cannot read plugin" style failures), so every one of them is
// pinned to the single copy that @tiptap/pm resolves to.
const prosemirrorPackages = [
  'prosemirror-model',
  'prosemirror-state',
  'prosemirror-view',
  'prosemirror-transform',
  'prosemirror-tables',
  'prosemirror-schema-list',
  'prosemirror-keymap',
  'prosemirror-commands',
  'prosemirror-history',
  'prosemirror-inputrules',
  'prosemirror-gapcursor',
  'prosemirror-dropcursor',
];

const singleInstanceAliases = {
  '@blocknote/core': require.resolve('@blocknote/core'),
  '@blocknote/react': require.resolve('@blocknote/react'),
  '@blocknote/shadcn': require.resolve('@blocknote/shadcn'),
  ...Object.fromEntries(
    prosemirrorPackages.map((pkg) => [pkg, resolveFromTiptapPm(pkg)])
  ),
};

/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: false, // Disabled for BlockNote compatibility
  output: 'export',
  images: {
    unoptimized: true,
  },
  // Add basePath configuration
  basePath: '',
  assetPrefix: '/',

  // Not decorative: with a custom `webpack()` function present, Next refuses
  // to start unless a bundler is chosen explicitly, and an empty `turbopack`
  // object is how that choice is expressed in config rather than by adding a
  // flag to every script.
  //
  // No `turbopack.resolveAlias` counterpart to the webpack aliases below:
  // Turbopack cannot resolve absolute Windows paths ("windows imports are not
  // implemented yet"), and the aliases are not actually needed for it. They
  // exist to collapse duplicate ProseMirror copies, and the lockfile already
  // installs exactly one copy of every @blocknote/* and prosemirror-* package,
  // so Turbopack resolves them to the same files on its own. If a second copy
  // of any of them ever appears in node_modules/.pnpm, dedupe it there (or via
  // a pnpm override) rather than by aliasing an absolute path here.
  turbopack: {},

  // Kept as an escape hatch: `next dev --webpack` / `next build --webpack`
  // still work and must resolve the same single ProseMirror instance.
  webpack: (config, { isServer }) => {
    if (!isServer) {
      config.resolve.fallback = {
        ...config.resolve.fallback,
        fs: false,
        path: false,
        os: false,
      };

      config.resolve.alias = {
        ...config.resolve.alias,
        // `$` = exact request only, so subpath imports such as
        // `@blocknote/core/style.css` keep resolving normally.
        '@blocknote/core$': singleInstanceAliases['@blocknote/core'],
        '@blocknote/react$': singleInstanceAliases['@blocknote/react'],
        '@blocknote/shadcn$': singleInstanceAliases['@blocknote/shadcn'],
        ...Object.fromEntries(
          prosemirrorPackages.map((pkg) => [pkg, singleInstanceAliases[pkg]])
        ),
      };
    }
    return config;
  },
}

module.exports = nextConfig
