import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  bitloopsSidebar: [
    'intro',
    {
      type: 'category',
      label: 'Getting Started',
      collapsed: false,
      items: [
        'getting-started/introduction',
        'getting-started/quickstart',
        'getting-started/key-features',
      ],
    },
    {
      type: 'category',
      label: 'Concepts',
      items: [
        'concepts/how-bitloops-works',
        'concepts/checkpoints-and-sessions',
        'concepts/intelligence-layer',
        'concepts/knowledge-store',
        'concepts/devql',
        'concepts/capability-packs',
        'concepts/capture',
      ],
    },
    {
      type: 'category',
      label: 'Guides',
      items: [
        'guides/end-to-end-workflow',
        'guides/team-setup',
        'guides/configuring-devql',
        'guides/devql-graphql',
        'guides/devql-query-cookbook',
        'guides/connecting-knowledge-sources',
        'guides/configuring-storage',
        'guides/using-the-dashboard',
        'guides/dashboard-local-https-setup',
      ],
    },
    {
      type: 'category',
      label: 'Reference',
      items: [
        'reference/cli-commands',
        'reference/uninstall',
        'reference/configuration',
        'reference/environment-variables',
      ],
    },
    {
      type: 'category',
      label: 'Troubleshooting',
      items: [
        'troubleshooting/common-issues',
        'troubleshooting/faq',
      ],
    },
  ],
};

export default sidebars;
