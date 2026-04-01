import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  contributorsSidebar: [
    'intro',
    {
      type: 'category',
      label: 'Getting Started',
      items: [
        'development-setup',
        'submitting-changes',
      ],
    },
    {
      type: 'category',
      label: 'Architecture',
      items: [
        'architecture',
        'architecture/layered-extension-architecture',
        'architecture/host-substrate',
        'architecture/capability-packs',
        'architecture/language-adapters',
        'architecture/agent-adapters',
        'architecture/devql-core-pack-boundaries',
        'architecture/decisions/graphql-first-devql-host-runtime',
      ],
    },
    {
      type: 'category',
      label: 'Extension Guides',
      items: [
        'guides/language-adapter-contributing',
        'guides/agent-extension-playbook',
        'guides/agent-family-profile-playbook',
        'guides/rust-code-standards',
      ],
    },
  ],
};

export default sidebars;
