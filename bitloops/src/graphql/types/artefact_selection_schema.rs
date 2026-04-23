pub(super) const CHECKPOINT_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  checkpoints(agent: String, since: DateTime): CheckpointStageResult!
}

type CheckpointStageResult {
  overview: JSON!
  schema: String
  items(first: Int! = 20): [Checkpoint!]!
}

type Checkpoint {
  id: ID!
  sessionId: String!
  commitSha: String
  branch: String
  agent: String
  eventTime: DateTime!
  strategy: String
  filesTouched: [String!]!
  payload: JSON
  commit: Commit
  fileRelations: [CheckpointFileRelation!]!
}"#;

pub(super) const CLONE_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  codeMatches(relationKind: String, minScore: Float): CloneStageResult!
}

type CloneStageResult {
  overview: JSON!
  schema: String
  items(first: Int! = 20): [Clone!]!
}

type Clone {
  id: ID!
  sourceArtefactId: ID!
  targetArtefactId: ID!
  relationKind: String!
  score: Float!
  metadata: JSON
  sourceArtefact: Artefact!
  targetArtefact: Artefact!
}"#;

pub(super) const DEPENDENCY_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  dependencies(kind: EdgeKind, direction: DepsDirection! = BOTH, includeUnresolved: Boolean! = true): DependencyStageResult!
}

type DependencyStageResult {
  overview: JSON!
  expandHint: DependencyExpandHint
  schema: String
  items(first: Int! = 20): [DependencyEdge!]!
}

type DependencyExpandHint {
  intent: String!
  template: String!
  parameters: DependencyExpandHintParameters!
}

type DependencyExpandHintParameters {
  direction: [DepsDirection!]!
  kind: [EdgeKind!]!
}

type DependencyEdge {
  id: ID!
  edgeKind: EdgeKind!
  fromArtefactId: ID!
  toArtefactId: ID
  toSymbolRef: String
  startLine: Int
  endLine: Int
  metadata: JSON
  fromArtefact: Artefact!
  toArtefact: Artefact
}"#;

pub(super) const TESTS_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  tests(minConfidence: Float, linkageSource: String): TestsStageResult!
}

type TestsStageResult {
  overview: JSON!
  schema: String
  items(first: Int! = 20): [TestHarnessTestsResult!]!
}

type TestHarnessTestsResult {
  artefact: TestHarnessArtefactRef!
  coveringTests: [TestHarnessCoveringTest!]!
  summary: TestHarnessTestsSummary!
}"#;
