pub(in crate::cli::devql::graphql) const INIT_SCHEMA_MUTATION: &str = r#"
    mutation InitSchema {
      initSchema {
        success
        repoIdentity
        repoId
        relationalBackend
        eventsBackend
      }
    }
"#;

pub(in crate::cli::devql::graphql) const ENQUEUE_TASK_MUTATION: &str = r#"
    mutation EnqueueTask($input: EnqueueTaskInput!) {
      enqueueTask(input: $input) {
        merged
        task {
          taskId
          repoId
          repoName
          repoIdentity
          kind
          source
          status
          submittedAtUnix
          startedAtUnix
          updatedAtUnix
          completedAtUnix
          queuePosition
          tasksAhead
          error
          syncSpec {
            mode
            paths
          }
          ingestSpec {
            backfill
          }
          embeddingsBootstrapSpec {
            configPath
            profileName
          }
          summaryBootstrapSpec {
            action
            message
            modelName
            gatewayUrlOverride
          }
          syncProgress {
            phase
            currentPath
            pathsTotal
            pathsCompleted
            pathsRemaining
            pathsUnchanged
            pathsAdded
            pathsChanged
            pathsRemoved
            cacheHits
            cacheMisses
            parseErrors
          }
          ingestProgress {
            phase
            commitsTotal
            commitsProcessed
            checkpointCompanionsProcessed
            currentCheckpointId
            currentCommitSha
            eventsInserted
            artefactsUpserted
          }
          embeddingsBootstrapProgress {
            phase
            assetName
            bytesDownloaded
            bytesTotal
            version
            message
          }
          summaryBootstrapProgress {
            phase
            assetName
            bytesDownloaded
            bytesTotal
            version
            message
          }
          syncResult {
            success
            mode
            parserVersion
            extractorVersion
            activeBranch
            headCommitSha
            headTreeSha
            pathsUnchanged
            pathsAdded
            pathsChanged
            pathsRemoved
            cacheHits
            cacheMisses
            parseErrors
            validation {
              valid
              expectedArtefacts
              actualArtefacts
              expectedEdges
              actualEdges
              missingArtefacts
              staleArtefacts
              mismatchedArtefacts
              missingEdges
              staleEdges
              mismatchedEdges
              filesWithDrift {
                path
                missingArtefacts
                staleArtefacts
                mismatchedArtefacts
                missingEdges
                staleEdges
                mismatchedEdges
              }
            }
          }
          ingestResult {
            success
            commitsProcessed
            checkpointCompanionsProcessed
            eventsInserted
            artefactsUpserted
            semanticFeatureRowsUpserted
            semanticFeatureRowsSkipped
            symbolEmbeddingRowsUpserted
            symbolEmbeddingRowsSkipped
            symbolCloneEdgesUpserted
            symbolCloneSourcesScored
          }
          embeddingsBootstrapResult {
            version
            binaryPath
            cacheDir
            runtimeName
            modelName
            freshlyInstalled
            message
          }
          summaryBootstrapResult {
            outcomeKind
            modelName
            message
          }
        }
      }
    }
"#;

pub(in crate::cli::devql::graphql) const PAUSE_TASK_QUEUE_MUTATION: &str = r#"
    mutation PauseTaskQueue($reason: String) {
      pauseTaskQueue(reason: $reason) {
        message
        repoId
        paused
        pausedReason
        updatedAtUnix
      }
    }
"#;

pub(in crate::cli::devql::graphql) const START_INIT_MUTATION: &str = r#"
    mutation StartInit($repoId: String!, $input: StartInitInput!) {
      startInit(repoId: $repoId, input: $input) {
        initSessionId
      }
    }
"#;

pub(in crate::cli::devql::graphql) const RESUME_TASK_QUEUE_MUTATION: &str = r#"
    mutation ResumeTaskQueue($repoId: String) {
      resumeTaskQueue(repoId: $repoId) {
        message
        repoId
        paused
        pausedReason
        updatedAtUnix
      }
    }
"#;

pub(in crate::cli::devql::graphql) const CANCEL_TASK_MUTATION: &str = r#"
    mutation CancelTask($id: String!) {
      cancelTask(id: $id) {
        taskId
        repoId
        repoName
        repoIdentity
        kind
        source
        status
        submittedAtUnix
        startedAtUnix
        updatedAtUnix
        completedAtUnix
        queuePosition
        tasksAhead
        error
        syncSpec {
          mode
          paths
        }
        ingestSpec {
          backfill
        }
        embeddingsBootstrapSpec {
          configPath
          profileName
        }
        syncProgress {
          phase
          currentPath
          pathsTotal
          pathsCompleted
          pathsRemaining
          pathsUnchanged
          pathsAdded
          pathsChanged
          pathsRemoved
          cacheHits
          cacheMisses
          parseErrors
        }
        ingestProgress {
          phase
          commitsTotal
          commitsProcessed
          checkpointCompanionsProcessed
          currentCheckpointId
          currentCommitSha
          eventsInserted
          artefactsUpserted
        }
        embeddingsBootstrapProgress {
          phase
          assetName
          bytesDownloaded
          bytesTotal
          version
          message
        }
        syncResult {
          success
          mode
        }
        ingestResult {
          success
          commitsProcessed
          eventsInserted
          artefactsUpserted
        }
            embeddingsBootstrapResult {
              version
              binaryPath
              cacheDir
              runtimeName
              modelName
              freshlyInstalled
              message
            }
            summaryBootstrapResult {
              outcomeKind
              modelName
              message
            }
      }
    }
"#;
