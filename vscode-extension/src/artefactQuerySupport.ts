import { BitloopsQueryClient, errorMentionsUnknownField } from './bitloopsCli';
import {
  ArtefactFieldSupport,
  DEFAULT_ARTEFACT_FIELD_SUPPORT,
} from './queryBuilder';

function downgradeFieldSupport(
  current: ArtefactFieldSupport,
  error: unknown,
): ArtefactFieldSupport | undefined {
  const next: ArtefactFieldSupport = {
    ...current,
  };
  let changed = false;

  if (current.summary && errorMentionsUnknownField(error, 'summary')) {
    next.summary = false;
    changed = true;
  }

  if (
    current.embeddingRepresentations &&
    errorMentionsUnknownField(error, 'embeddingRepresentations')
  ) {
    next.embeddingRepresentations = false;
    changed = true;
  }

  return changed ? next : undefined;
}

export class ArtefactQuerySupport {
  private fieldSupport: ArtefactFieldSupport = {
    ...DEFAULT_ARTEFACT_FIELD_SUPPORT,
  };

  current(): ArtefactFieldSupport {
    return {
      ...this.fieldSupport,
    };
  }

  async executeWithFallback<T>(
    client: BitloopsQueryClient,
    cwd: string,
    buildQuery: (fieldSupport: ArtefactFieldSupport) => string,
  ): Promise<T> {
    let fieldSupport = this.current();

    for (;;) {
      try {
        const result = await client.executeGraphqlQuery<T>(cwd, buildQuery(fieldSupport));
        this.fieldSupport = fieldSupport;
        return result;
      } catch (error) {
        const downgraded = downgradeFieldSupport(fieldSupport, error);
        if (!downgraded) {
          throw error;
        }

        fieldSupport = downgraded;
      }
    }
  }
}
