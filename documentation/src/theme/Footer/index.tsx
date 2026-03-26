import React from 'react';
import styles from './styles.module.css';

const footerLinks = {
  agents: {
    title: 'Agents',
    items: [
      {label: 'Claude Code', href: 'https://bitloops.com/claude-code'},
      {label: 'Codex', href: 'https://bitloops.com/codex'},
      {label: 'GitHub Copilot', href: 'https://bitloops.com/copilot'},
      {label: 'Cursor', href: 'https://bitloops.com/cursor'},
      {label: 'Gemini', href: 'https://bitloops.com/gemini'},
      {label: 'OpenCode', href: 'https://bitloops.com/opencode'},
    ],
  },
  resources: {
    title: 'Resources',
    items: [
      {label: 'Context Engineering', href: 'https://bitloops.com/resources/context-engineering'},
      {label: 'Software Architecture', href: 'https://bitloops.com/resources/software-architecture'},
      {label: 'AI Agent Infrastructure', href: 'https://bitloops.com/resources/agent-tooling'},
      {label: 'AI Native Development', href: 'https://bitloops.com/resources/ai-native-software-development'},
      {label: 'AI Code Governance', href: 'https://bitloops.com/resources/ai-code-governance'},
      {label: 'Browse all Hub topics \u2192', href: 'https://bitloops.com/resources'},
    ],
  },
  product: {
    title: 'Product',
    items: [
      {label: 'Home', href: 'https://bitloops.com'},
      {label: 'GitHub', href: 'https://github.com/bitloops'},
      {label: 'Docs', href: '/docs/'},
      {label: 'Blog', href: 'https://bitloops.com/blog'},
    ],
  },
  company: {
    title: 'Company',
    items: [
      {label: 'About us', href: 'https://bitloops.com/about-us'},
      {label: 'LinkedIn', href: 'https://www.linkedin.com/company/bitloops/'},
      {label: 'Discord', href: 'https://discord.com/invite/vj8EdZx8gK'},
    ],
  },
  legal: {
    title: 'Legal',
    items: [
      {label: 'Privacy Policy', href: 'https://bitloops.com/privacy'},
      {label: 'Terms of Service', href: 'https://bitloops.com/terms'},
    ],
  },
};

function FooterLinkColumn({title, items}: {title: string; items: {label: string; href: string}[]}) {
  return (
    <div className={styles.column}>
      <h4 className={styles.columnTitle}>{title}</h4>
      <ul className={styles.linkList}>
        {items.map((item) => (
          <li key={item.label}>
            <a href={item.href} className={styles.link} target={item.href.startsWith('http') ? '_blank' : undefined} rel={item.href.startsWith('http') ? 'noopener noreferrer' : undefined}>
              {item.label}
            </a>
          </li>
        ))}
      </ul>
    </div>
  );
}

export default function Footer(): React.JSX.Element {
  return (
    <footer className={styles.footer}>
      <div className={styles.container}>
        <div className={styles.top}>
          <div className={styles.brand}>
            <img
              src="/docs/img/bitloops-logo-dark-bg.svg"
              alt="Bitloops"
              className={styles.logo}
            />
            <p className={styles.tagline}>
              The open-source intelligence layer for AI-native development.
            </p>
          </div>
          <div className={styles.linkColumns}>
            {Object.values(footerLinks).map((section) => (
              <FooterLinkColumn key={section.title} title={section.title} items={section.items} />
            ))}
          </div>
        </div>
        <div className={styles.divider} />
        <div className={styles.bottom}>
          <span className={styles.copyright}>
            Copyright &copy; {new Date().getFullYear()}. All rights reserved by Bitloops.
          </span>
        </div>
      </div>
    </footer>
  );
}
