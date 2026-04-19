pub(in crate::cli::devql::graphql) const RUNTIME_EVENTS_SUBSCRIPTION: &str = r#"
    subscription RuntimeEvents($repoId: String!, $initSessionId: ID) {
      runtimeEvents(repoId: $repoId, initSessionId: $initSessionId) {
        domain
        repoId
        initSessionId
        updatedAtUnix
        taskId
        runId
        mailboxName
      }
    }
"#;

pub(in crate::cli::devql::graphql) const TASK_PROGRESS_SUBSCRIPTION: &str = r#"
    subscription TaskProgress($taskId: String!) {
      taskProgress(taskId: $taskId) {
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
        }
      }
    }
"#;
