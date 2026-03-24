import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'Bitloops Docs',
  tagline: 'The local-first memory and context layer for AI coding agents',
  favicon: 'img/favicon.ico',

  future: {
    v4: true,
  },

  url: 'https://bitloops.com',
  baseUrl: '/docs/',

  organizationName: 'bitloops',
  projectName: 'bitloops',

  onBrokenLinks: 'throw',

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      {
        docs: {
          sidebarPath: './sidebars.ts',
          routeBasePath: '/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  plugins: [
    [
      '@docusaurus/plugin-content-docs',
      {
        id: 'contributors',
        path: 'contributors',
        routeBasePath: 'contributors',
        sidebarPath: './sidebarsContributors.ts',
      },
    ],
  ],

  themeConfig: {
    image: 'img/Bitloops-Logo.svg',
    colorMode: {
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'Bitloops',
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'bitloopsSidebar',
          position: 'left',
          label: 'Documentation',
        },
        {
          type: 'docSidebar',
          sidebarId: 'contributorsSidebar',
          position: 'left',
          label: 'Contributors',
          docsPluginId: 'contributors',
        },
        {
          href: 'https://github.com/bitloops/bitloops',
          label: 'GitHub',
          position: 'right',
        },
        {
          href: 'https://bitloops.com',
          label: 'Home',
          position: 'right',
        },
      ],
    },
    // Footer is rendered by custom component at src/theme/Footer
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
