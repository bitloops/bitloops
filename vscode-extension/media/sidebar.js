(function () {
  const vscode = acquireVsCodeApi();
  const app = document.getElementById('app');
  let state = {
    query: '',
    loading: false,
    searchMode: undefined,
    results: [],
    searchSections: [],
    totalCount: 0,
    breadcrumbs: [],
  };
  let draftQuery = '';

  function escapeHtml(value) {
    return String(value)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function pluralise(count, singular, plural) {
    return `${count} ${count === 1 ? singular : plural || singular + 's'}`;
  }

  function formatSearchMode(mode) {
    if (!mode || typeof mode !== 'string') {
      return '';
    }

    return mode.charAt(0) + mode.slice(1).toLowerCase();
  }

  function renderBadge(badge) {
    const statusLabel = badge.available ? 'available' : 'missing';
    return `<span
      class="badge ${badge.available ? 'badge--available' : 'badge--missing'}"
      title="${escapeHtml(`${badge.label} embedding ${statusLabel}`)}"
      aria-label="${escapeHtml(`${badge.label} embedding ${statusLabel}`)}"
    >${escapeHtml(badge.label)}</span>`;
  }

  function renderChip(chip) {
    return `<button
      class="chip ${chip.active ? 'chip--active' : ''}"
      data-action="open-stage"
      data-stage="${escapeHtml(chip.stage)}"
      ${chip.filterKey ? `data-filter-key="${escapeHtml(chip.filterKey)}"` : ''}
      ${chip.disabled ? 'disabled' : ''}
      type="button"
    >
      <span>${escapeHtml(chip.label)}</span>
      <strong>${chip.count}</strong>
    </button>`;
  }

  function renderResultCard(result) {
    return `<button class="result-card" data-action="select-result" data-result-id="${escapeHtml(
      result.id,
    )}" type="button">
      <span class="result-card__title">${escapeHtml(result.title)}</span>
      <span class="result-card__description">${escapeHtml(result.description)}</span>
      ${
        result.scoreLabel
          ? `<span class="result-card__metrics">${escapeHtml(result.scoreLabel)}</span>`
          : ''
      }
      ${
        result.scoreBreakdownLabel
          ? `<span class="result-card__metrics result-card__metrics--detail">${escapeHtml(
              result.scoreBreakdownLabel,
            )}</span>`
          : ''
      }
      ${
        result.matchBreakdownLabel
          ? `<span class="result-card__metrics result-card__metrics--detail">${escapeHtml(
              result.matchBreakdownLabel,
            )}</span>`
          : ''
      }
      ${
        result.summaryPreview
          ? `<span class="result-card__summary">${escapeHtml(result.summaryPreview)}</span>`
          : ''
      }
    </button>`;
  }

  function renderSearchSection(section) {
    return `<section class="panel">
      <div class="panel__header">
        <h2>${escapeHtml(section.title)}</h2>
        <span class="meta">${pluralise(section.results.length, 'result')}</span>
      </div>
      ${
        section.description
          ? `<p class="panel__copy">${escapeHtml(section.description)}</p>`
          : ''
      }
      <div class="results-list">
        ${section.results.map(renderResultCard).join('')}
      </div>
    </section>`;
  }

  function renderResults() {
    if (!state.results || state.results.length === 0) {
      return `<div class="empty-state">
        <h2>Search Bitloops artefacts</h2>
        <p>Use the search box to query exact lexical and conceptual matches in the active workspace folder.</p>
      </div>`;
    }

    const modeLabel = formatSearchMode(state.searchMode);
    const resultMeta =
      state.totalCount > state.results.length
        ? `Showing ${state.results.length} of ${state.totalCount}`
        : pluralise(state.totalCount, 'result');
    const headerMeta = modeLabel ? `${resultMeta} · ${modeLabel}` : resultMeta;

    return `<div class="search-results-shell">
      <section class="panel">
        <div class="panel__header">
          <h2>Results</h2>
          <span class="meta">${escapeHtml(headerMeta)}</span>
        </div>
        <div class="results-list">
          ${state.results.map(renderResultCard).join('')}
        </div>
      </section>
      ${(state.searchSections || []).map(renderSearchSection).join('')}
    </div>`;
  }

  function renderSelection() {
    if (!state.selection) {
      return '';
    }

    const overviewSegments =
      state.selection.overviewSegments && state.selection.overviewSegments.length > 0
        ? state.selection.overviewSegments
            .map((segment) => `<span class="meta-pill">${escapeHtml(segment)}</span>`)
            .join('')
        : '<span class="meta-pill">No related data</span>';

    const stageSection = state.stage
      ? `<section class="panel">
          <div class="panel__header">
            <h2>${escapeHtml(state.stage.title)}</h2>
            <span class="meta">${pluralise(state.stage.rows.length, 'row')}</span>
          </div>
          ${
            state.stage.rows.length === 0
              ? `<p class="empty-copy">${escapeHtml(state.stage.emptyMessage)}</p>`
              : `<div class="rows-list">
                  ${state.stage.rows
                    .map(
                      (row) => `<button class="row-card" data-action="activate-row" data-row-id="${escapeHtml(
                        row.id,
                      )}" type="button">
                      <span class="row-card__title">${escapeHtml(row.title)}</span>
                      ${row.description ? `<span class="row-card__description">${escapeHtml(row.description)}</span>` : ''}
                      ${row.detail ? `<span class="row-card__detail">${escapeHtml(row.detail)}</span>` : ''}
                    </button>`,
                    )
                    .join('')}
                </div>`
          }
        </section>`
      : `<section class="panel panel--hint">
          <div class="panel__header">
            <h2>Related results</h2>
          </div>
          <p class="empty-copy">Choose a stage above to load dependencies, code matches, tests, or checkpoints for this selection.</p>
        </section>`;

    const checkpointSection = state.checkpoint
      ? `<section class="panel">
          <div class="panel__header">
            <h2>${escapeHtml(state.checkpoint.title)}</h2>
          </div>
          ${
            state.checkpoint.description
              ? `<p class="checkpoint-copy">${escapeHtml(state.checkpoint.description)}</p>`
              : ''
          }
          ${
            state.checkpoint.metadata.length > 0
              ? `<div class="metadata-list">${state.checkpoint.metadata
                  .map((item) => `<span class="meta-pill">${escapeHtml(item)}</span>`)
                  .join('')}</div>`
              : ''
          }
          <div class="checkpoint-files">
            ${
              state.checkpoint.files.length > 0
                ? state.checkpoint.files
                    .map(
                      (file) => `<button class="file-link" data-action="open-checkpoint-file" data-row-id="${escapeHtml(
                        state.checkpoint.id,
                      )}" data-file-id="${escapeHtml(file.id)}" type="button">
                        <span>${escapeHtml(file.label)}</span>
                        ${
                          file.changeKind
                            ? `<strong>${escapeHtml(file.changeKind.toLowerCase())}</strong>`
                            : ''
                        }
                      </button>`,
                    )
                    .join('')
                : '<p class="empty-copy">No touched files were recorded for this checkpoint.</p>'
            }
          </div>
        </section>`
      : '';

    return `<section class="selection-shell">
      <section class="panel panel--hero">
        <div class="panel__header panel__header--stacked">
          <div>
            <p class="eyebrow">Inspector</p>
            <h2>${escapeHtml(state.selection.title)}</h2>
            ${
              state.selection.subtitle
                ? `<p class="selection-subtitle">${escapeHtml(state.selection.subtitle)}</p>`
                : ''
            }
          </div>
          <button class="secondary-button" data-action="open-selection" type="button">Open in editor</button>
        </div>
        ${
          state.selection.summary
            ? `<div class="summary-block">
                <h3>Summary</h3>
                <p>${escapeHtml(state.selection.summary)}</p>
              </div>`
            : '<div class="summary-block summary-block--empty"><h3>Summary</h3><p>No summary is available for this artefact.</p></div>'
        }
        <div class="overview-block">
          <div class="overview-block__header">
            <h3>${escapeHtml(state.selection.overviewTitle)}</h3>
            <span class="meta">${escapeHtml(state.selection.openInEditorLabel || '')}</span>
          </div>
          <div class="metadata-list">${overviewSegments}</div>
        </div>
        <div class="embedding-block">
          <h3>Embeddings</h3>
          <div class="badge-row">${state.selection.badges.map(renderBadge).join('')}</div>
        </div>
        <div class="chip-row">
          ${state.selection.chips.map(renderChip).join('')}
        </div>
      </section>
      ${checkpointSection || stageSection}
    </section>`;
  }

  function renderStatus() {
    if (state.loading && state.loadingLabel) {
      return `<div class="status-banner status-banner--loading">${escapeHtml(state.loadingLabel)}</div>`;
    }

    if (state.statusMessage) {
      return `<div class="status-banner">${escapeHtml(state.statusMessage)}</div>`;
    }

    return '';
  }

  function renderBreadcrumbs() {
    if (!state.breadcrumbs || state.breadcrumbs.length === 0) {
      return '';
    }

    return `<nav class="breadcrumbs">
      ${state.breadcrumbs
        .map(
          (crumb) => `<button
            class="breadcrumb ${crumb.active ? 'breadcrumb--active' : ''}"
            data-action="breadcrumb"
            data-id="${escapeHtml(crumb.id)}"
            type="button"
          >${escapeHtml(crumb.label)}</button>`,
        )
        .join('<span class="breadcrumb-separator">/</span>')}
    </nav>`;
  }

  function render() {
    app.innerHTML = `
      <section class="search-shell">
        <form class="search-bar" id="search-form">
          <label class="search-bar__label" for="search-input">Search</label>
          <div class="search-bar__controls">
            <input id="search-input" class="search-input" type="text" value="${escapeHtml(
              draftQuery,
            )}" placeholder="Search DevQL artefacts" />
            <button class="primary-button" type="submit">Search</button>
            <button class="secondary-button" data-action="refresh" type="button">Refresh</button>
            ${
              state.selection || state.stage || state.checkpoint
                ? '<button class="secondary-button" data-action="back" type="button">Back</button>'
                : ''
            }
          </div>
        </form>
        ${renderBreadcrumbs()}
        ${renderStatus()}
        <main class="content-shell">
          ${state.selection ? renderSelection() : renderResults()}
        </main>
      </section>
    `;
  }

  window.addEventListener('message', (event) => {
    const message = event.data;
    if (!message || typeof message !== 'object') {
      return;
    }

    if (message.type === 'setState') {
      const activeElement = document.activeElement;
      if (!activeElement || activeElement.id !== 'search-input') {
        draftQuery = message.state.query || '';
      }

      state = message.state;
      render();
      return;
    }

    if (message.type === 'focusSearch') {
      const input = document.getElementById('search-input');
      if (input) {
        input.focus();
        input.select();
      }
    }
  });

  document.addEventListener('input', (event) => {
    if (event.target && event.target.id === 'search-input') {
      draftQuery = event.target.value;
    }
  });

  document.addEventListener('submit', (event) => {
    if (event.target && event.target.id === 'search-form') {
      event.preventDefault();
      vscode.postMessage({
        type: 'search',
        query: draftQuery,
      });
    }
  });

  document.addEventListener('click', (event) => {
    const target = event.target instanceof Element ? event.target.closest('[data-action]') : null;
    if (!target) {
      return;
    }

    const action = target.getAttribute('data-action');
    switch (action) {
      case 'refresh':
        vscode.postMessage({ type: 'refresh' });
        break;
      case 'back':
        vscode.postMessage({ type: 'back' });
        break;
      case 'breadcrumb':
        vscode.postMessage({
          type: 'breadcrumb',
          id: target.getAttribute('data-id'),
        });
        break;
      case 'select-result':
        vscode.postMessage({
          type: 'selectResult',
          resultId: target.getAttribute('data-result-id'),
        });
        break;
      case 'open-stage':
        vscode.postMessage({
          type: 'openStage',
          stage: target.getAttribute('data-stage'),
          filterKey: target.getAttribute('data-filter-key') || undefined,
        });
        break;
      case 'activate-row':
        vscode.postMessage({
          type: 'activateRow',
          rowId: target.getAttribute('data-row-id'),
        });
        break;
      case 'open-selection':
        vscode.postMessage({
          type: 'openSelectionInEditor',
        });
        break;
      case 'open-checkpoint-file':
        vscode.postMessage({
          type: 'openCheckpointFile',
          rowId: target.getAttribute('data-row-id'),
          fileId: target.getAttribute('data-file-id'),
        });
        break;
      default:
        break;
    }
  });

  vscode.postMessage({ type: 'ready' });
})();
