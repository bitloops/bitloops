pub(in crate::graphql::types::artefact_selection) const CHECKPOINT_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
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

pub(in crate::graphql::types::artefact_selection) const CLONE_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  codeMatches(relationKind: String, minScore: Float): CloneStageResult!
}

type CloneStageResult {
  overview: JSON!
  expandHint: CloneExpandHint
  schema: String
  items(first: Int! = 20): [Clone!]!
}

interface ExpandHint {
  intent: String!
  template: String!
  parameters: [ExpandHintParameter!]!
}

type ExpandHintParameter {
  name: String!
  intent: String!
  supportedValues: [String!]!
}

type CloneExpandHint implements ExpandHint {
  intent: String!
  template: String!
  parameters: [ExpandHintParameter!]!
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

pub(in crate::graphql::types::artefact_selection) const DEPENDENCY_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  dependencies(kind: EdgeKind, direction: DependenciesDirection! = BOTH, includeUnresolved: Boolean! = true): DependencyStageResult!
}

type DependencyStageResult {
  overview: JSON!
  expandHint: DependencyExpandHint
  schema: String
  items(first: Int! = 20): [DependencyEdge!]!
}

interface ExpandHint {
  intent: String!
  template: String!
  parameters: [ExpandHintParameter!]!
}

type ExpandHintParameter {
  name: String!
  intent: String!
  supportedValues: [String!]!
}

type DependencyExpandHint implements ExpandHint {
  intent: String!
  template: String!
  parameters: [ExpandHintParameter!]!
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

pub(in crate::graphql::types::artefact_selection) const TESTS_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
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

pub(in crate::graphql::types::artefact_selection) const HISTORICAL_CONTEXT_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  historicalContext(agent: String, since: DateTime, evidenceKind: HistoricalEvidenceKind): HistoricalContextStageResult!
}

type HistoricalContextStageResult {
  overview: JSON!
  schema: String
  items(first: Int! = 20): [HistoricalContextItem!]!
}

enum HistoricalEvidenceKind {
  SYMBOL_PROVENANCE
  FILE_RELATION
  LINE_OVERLAP
}

type HistoricalContextItem {
  checkpointId: ID!
  sessionId: String!
  turnId: String
  agentType: String
  model: String
  eventTime: DateTime!
  matchReason: HistoricalMatchReason!
  matchStrength: HistoricalMatchStrength!
  promptPreview: String
  turnSummary: String
  transcriptPreview: String
  filesModified: [String!]!
  fileRelations: [CheckpointFileRelation!]!
  toolEvents: [HistoricalToolEvent!]!
}

enum HistoricalMatchReason {
  SYMBOL_PROVENANCE
  FILE_RELATION
  LINE_OVERLAP
}

enum HistoricalMatchStrength {
  HIGH
  MEDIUM
  LOW
}

type HistoricalToolEvent {
  toolKind: String
  inputSummary: String
  outputSummary: String
  command: String
}"#;

pub(in crate::graphql::types::artefact_selection) const CONTEXT_GUIDANCE_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  contextGuidance(agent: String, since: DateTime, evidenceKind: HistoricalEvidenceKind, category: ContextGuidanceCategory, kind: String): ContextGuidanceStageResult!
}

type ContextGuidanceStageResult {
  overview: JSON!
  schema: String
  items(first: Int! = 20): [ContextGuidanceItem!]!
}

type ContextGuidanceItem {
  id: ID!
  category: ContextGuidanceCategory!
  kind: String!
  label: String!
  guidance: String!
  evidenceExcerpt: String!
  confidence: ContextGuidanceConfidence!
  relevanceScore: Float!
  generatedAt: DateTime
  sourceModel: String
  sourceCount: Int!
  sources: [ContextGuidanceSource!]!
}

type ContextGuidanceSource {
  sourceType: String!
  sourceId: String!
  checkpointId: ID
  sessionId: String
  turnId: String
  toolKind: String
  knowledgeItemId: ID
  knowledgeItemVersionId: ID
  relationAssertionId: ID
  provider: String
  sourceKind: String
  title: String
  url: String
  excerpt: String
}

enum ContextGuidanceCategory {
  DECISION
  CONSTRAINT
  PATTERN
  RISK
  VERIFICATION
  CONTEXT
}

enum ContextGuidanceConfidence {
  HIGH
  MEDIUM
  LOW
}"#;

pub(in crate::graphql::types::artefact_selection) const ARCHITECTURE_OVERVIEW_SCHEMA: &str = r#"type ArtefactSelectionOverview {
  architecture: ArchitectureOverviewStage!
}

type ArchitectureOverviewStage {
  overview: JSON!
  expandHint: JSON
  schema: String
}

type ArchitectureOverview {
  available: Boolean!
  reason: String
  selectedArtefactCount: Int!
  assignedSelectedArtefactCount: Int!
  unassignedSelectedArtefactCount: Int!
  roleAssignmentCount: Int!
  roleCount: Int!
  familyCounts: JSON!
  sourceCounts: JSON!
  targetKindCounts: JSON!
  confidence: JSON
  primaryRoles: [JSON!]!
  graphContextAvailable: Boolean!
}"#;

pub(in crate::graphql::types::artefact_selection) const ARCHITECTURE_ROLE_STAGE_SCHEMA: &str = r#"type ArtefactSelection {
  architectureRoles(first: Int! = 20): ArchitectureRoleStageResult!
}

type ArchitectureRoleStageResult {
  overview: JSON!
  expandHint: JSON
  schema: String
  items(first: Int! = 20): [ArchitectureRoleAssignmentItem!]!
}

type ArchitectureRoleAssignmentItem {
  assignmentId: String!
  role: ArchitectureRoleInfo!
  target: ArchitectureRoleTarget!
  priority: String!
  status: String!
  source: String!
  confidence: Float!
  classifierVersion: String!
  ruleVersion: Int
}

type ArchitectureRoleInfo {
  roleId: String!
  canonicalKey: String!
  displayName: String!
  family: String
  description: String!
}

type ArchitectureRoleTarget {
  targetKind: String!
  path: String!
  artefactId: String
  symbolId: String
  symbolFqn: String
  canonicalKind: String
}"#;

pub(in crate::graphql::types::artefact_selection) const ARCHITECTURE_GRAPH_CONTEXT_STAGE_SCHEMA:
    &str = r#"type ArtefactSelection {
  architectureGraphContext(nodeFirst: Int! = 20, edgeFirst: Int! = 20): ArchitectureGraphContextStageResult!
}

type ArchitectureGraphContextStageResult {
  overview: JSON!
  schema: String
  nodes(first: Int! = 20): [ArchitectureGraphNode!]!
  edges(first: Int! = 20): [ArchitectureGraphEdge!]!
}"#;
