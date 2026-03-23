import type {ReactNode} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import Heading from '@theme/Heading';
import styles from './styles.module.css';

type FeatureItem = {
  title: string;
  icon: string;
  description: ReactNode;
  link: string;
};

const FeatureList: FeatureItem[] = [
  {
    title: 'Quickstart',
    icon: '\u{1F680}',
    description: (
      <>
        Install Bitloops and capture your first AI session checkpoint
        in under 5 minutes.
      </>
    ),
    link: '/getting-started/quickstart',
  },
  {
    title: 'How It Works',
    icon: '\u{1F4A1}',
    description: (
      <>
        Understand the architecture: hooks, checkpoints, DevQL, and
        the local knowledge graph.
      </>
    ),
    link: '/concepts/how-bitloops-works',
  },
  {
    title: 'End-to-End Workflow',
    icon: '\u{1F4D6}',
    description: (
      <>
        See a complete real-world scenario from AI prompt to
        checkpoint to code review.
      </>
    ),
    link: '/guides/end-to-end-workflow',
  },
  {
    title: 'Checkpoints & Sessions',
    icon: '\u{1F4CB}',
    description: (
      <>
        Understand Draft Commits, Committed Checkpoints, and how
        Bitloops makes AI reasoning traceable.
      </>
    ),
    link: '/concepts/checkpoints-and-sessions',
  },
  {
    title: 'Team Setup',
    icon: '\u{1F465}',
    description: (
      <>
        Share AI reasoning through git. Onboard teammates and
        configure shared settings.
      </>
    ),
    link: '/guides/team-setup',
  },
  {
    title: 'CLI Reference',
    icon: '\u{2328}\u{FE0F}',
    description: (
      <>
        Every command, flag, and option with example output.
      </>
    ),
    link: '/reference/cli-commands',
  },
];

function Feature({title, icon, description, link}: FeatureItem) {
  return (
    <div className={clsx('col col--4')}>
      <Link to={link} className={styles.featureLink}>
        <div className="feature-card">
          <div className="feature-card__icon">{icon}</div>
          <Heading as="h3">{title}</Heading>
          <p>{description}</p>
        </div>
      </Link>
    </div>
  );
}

export default function HomepageFeatures(): ReactNode {
  return (
    <section className={styles.features}>
      <div className="container">
        <div className="row">
          {FeatureList.map((props, idx) => (
            <Feature key={idx} {...props} />
          ))}
        </div>
      </div>
    </section>
  );
}
