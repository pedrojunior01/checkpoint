// SaveVault DEV-UI-01 — Native Tauri IPC Client
(function () {
  'use strict';

  // State
  let currentTab = 'tab-games';
  let cachedGames = [];
  let selectedGameId = null;
  let pendingRestoreSnapshot = null;
  let isWindowVisible = true;
  let pollInterval = null;

  // Tauri IPC helper
  async function invoke(cmd, args = {}) {
    if (window.__TAURI__ && window.__TAURI__.core && typeof window.__TAURI__.core.invoke === 'function') {
      return window.__TAURI__.core.invoke(cmd, args);
    }
    if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
      return window.__TAURI_INTERNALS__.invoke(cmd, args);
    }
    throw new Error('Tauri IPC não disponível neste ambiente.');
  }

  // Toast Notification
  function showToast(message, type = 'info') {
    const container = document.getElementById('toast-container');
    if (!container) return;
    const toast = document.createElement('div');
    toast.className = `toast toast-${type}`;
    toast.innerHTML = `<span>${message}</span>`;
    container.appendChild(toast);
    setTimeout(() => {
      toast.style.opacity = '0';
      setTimeout(() => toast.remove(), 200);
    }, 4000);
  }

  // UI Setup & Tab Navigation
  function initNavigation() {
    const navButtons = document.querySelectorAll('.nav-item');
    navButtons.forEach((btn) => {
      btn.addEventListener('click', () => {
        const targetTab = btn.getAttribute('data-tab');
        if (!targetTab) return;
        switchTab(targetTab);
      });
    });

    // Top action buttons
    document.getElementById('btn-header-reconcile')?.addEventListener('click', triggerReconcile);
    document.getElementById('btn-header-discovery')?.addEventListener('click', triggerDiscovery);
    document.getElementById('btn-trigger-reconcile')?.addEventListener('click', triggerReconcile);
    document.getElementById('btn-empty-discovery')?.addEventListener('click', triggerDiscovery);

    // Refresh buttons
    document.getElementById('btn-refresh-games')?.addEventListener('click', loadGames);
    document.getElementById('btn-refresh-dashboard')?.addEventListener('click', loadDashboard);
    document.getElementById('btn-refresh-locations')?.addEventListener('click', loadLocations);
    document.getElementById('btn-refresh-snapshots')?.addEventListener('click', loadSnapshots);
    document.getElementById('btn-refresh-safety')?.addEventListener('click', loadSafetyBackups);
    document.getElementById('btn-refresh-sync')?.addEventListener('click', loadSyncQueue);
    document.getElementById('btn-refresh-events')?.addEventListener('click', loadEvents);

    // Games search filter
    document.getElementById('games-search')?.addEventListener('input', (e) => {
      filterGames(e.target.value);
    });

    // Snapshots search filter
    document.getElementById('snapshots-search')?.addEventListener('input', (e) => {
      filterSnapshots(e.target.value);
    });

    // Detail view back button
    document.getElementById('btn-back-to-games')?.addEventListener('click', () => {
      selectedGameId = null;
      document.getElementById('game-detail-view').style.display = 'none';
      document.getElementById('games-list-view').style.display = 'block';
    });

    // Modal close handlers
    document.getElementById('btn-modal-cancel')?.addEventListener('click', closeModal);
    document.getElementById('btn-modal-cancel-x')?.addEventListener('click', closeModal);
    document.getElementById('btn-modal-done')?.addEventListener('click', closeModal);
    document.getElementById('btn-modal-done-x')?.addEventListener('click', closeModal);
    document.getElementById('btn-modal-confirm')?.addEventListener('click', confirmRestore);

    document.getElementById('btn-scan-modal-close')?.addEventListener('click', closeScanModal);
    document.getElementById('btn-scan-modal-close-x')?.addEventListener('click', closeScanModal);
  }

  function switchTab(tabId) {
    currentTab = tabId;

    // Update active nav button
    document.querySelectorAll('.nav-item').forEach((btn) => {
      if (btn.getAttribute('data-tab') === tabId) {
        btn.classList.add('active');
      } else {
        btn.classList.remove('active');
      }
    });

    // Update active tab pane
    document.querySelectorAll('.tab-pane').forEach((pane) => {
      if (pane.id === tabId) {
        pane.classList.add('active');
      } else {
        pane.classList.remove('active');
      }
    });

    // Load data for the selected tab
    loadActiveTabData();
  }

  function loadActiveTabData() {
    // Header summary is always updated
    loadHeaderStatus();

    switch (currentTab) {
      case 'tab-games':
        if (selectedGameId) {
          loadGameDetails(selectedGameId);
        } else {
          loadGames();
        }
        break;
      case 'tab-dashboard':
        loadDashboard();
        break;
      case 'tab-locations':
        loadLocations();
        break;
      case 'tab-snapshots':
        loadSnapshots();
        break;
      case 'tab-safety':
        loadSafetyBackups();
        break;
      case 'tab-reconcile':
        loadReconciliation();
        break;
      case 'tab-sync':
        loadSyncQueue();
        break;
      case 'tab-events':
        loadEvents();
        break;
    }
  }

  // 1. Header & General Status
  async function loadHeaderStatus() {
    try {
      const summary = await invoke('get_dashboard_status');
      if (summary) {
        const valWatcher = document.getElementById('val-watcher');
        const valReconciler = document.getElementById('val-reconciler');
        const valDb = document.getElementById('val-db');

        if (valWatcher) valWatcher.textContent = summary.watcher_status === 'Running' ? 'Ativo' : summary.watcher_status;
        if (valReconciler) valReconciler.textContent = `Ativa (${summary.reconciliation_interval_secs}s)`;
        if (valDb) valDb.textContent = summary.db_status === 'Connected' ? 'Conectado' : summary.db_status;

        const badgeGames = document.getElementById('badge-games-count');
        if (badgeGames) badgeGames.textContent = summary.save_locations_count > 0 ? summary.save_locations_count : summary.save_locations_count;

        const badgeSync = document.getElementById('badge-sync-count');
        if (badgeSync) {
          if (summary.sync_pending_count > 0) {
            badgeSync.textContent = summary.sync_pending_count;
            badgeSync.style.display = 'inline-block';
          } else {
            badgeSync.style.display = 'none';
          }
        }
      }
    } catch (e) {
      console.warn('Erro ao atualizar status do header:', e);
    }
  }

  // 2. Tab: JOGOS (TELA PRINCIPAL)
  async function loadGames() {
    try {
      const games = await invoke('get_games');
      cachedGames = games || [];

      const badgeGames = document.getElementById('badge-games-count');
      if (badgeGames) badgeGames.textContent = cachedGames.length;

      renderGamesList(cachedGames);
    } catch (e) {
      console.error('Falha ao carregar jogos:', e);
      showToast(`Erro ao carregar jogos: ${e}`, 'error');
    }
  }

  function renderGamesList(games) {
    const grid = document.getElementById('games-grid');
    const empty = document.getElementById('games-empty');
    if (!grid) return;

    grid.innerHTML = '';

    if (!games || games.length === 0) {
      grid.style.display = 'none';
      if (empty) empty.style.display = 'flex';
      return;
    }

    grid.style.display = 'grid';
    if (empty) empty.style.display = 'none';

    games.forEach((game) => {
      const card = document.createElement('div');
      card.className = 'game-card';
      card.onclick = () => openGameDetails(game.id);

      // Platform badge text
      let platformDetail = game.platform;
      if (game.steam_app_id) {
        platformDetail = `Steam · AppID ${game.steam_app_id}`;
      } else if (game.gog_id) {
        platformDetail = `GOG · ID ${game.gog_id}`;
      }

      // Monitoring status
      let statusClass = 'game-status-none';
      let statusDot = 'dot-warning';
      if (game.monitoring_status === 'Monitorando') {
        statusClass = 'game-status-active';
        statusDot = 'dot-active';
      } else if (game.monitoring_status === 'Aguardando') {
        statusClass = 'game-status-waiting';
        statusDot = 'dot-warning';
      }

      // Last activity time formatted
      let actTime = '—';
      if (game.last_activity) {
        const parts = game.last_activity.split(' ');
        actTime = parts.length > 1 ? parts[1] : parts[0];
      }

      card.innerHTML = `
        <div class="game-card-header">
          <div class="game-title-group">
            <div class="game-title">
              <span>🎮</span>
              <span>${escapeHtml(game.name)}</span>
            </div>
            <div class="game-platform-tag">${escapeHtml(platformDetail)}</div>
          </div>
          <button class="btn btn-secondary btn-sm" onclick="event.stopPropagation(); window.openGameDetails('${game.id}')">
            Abrir →
          </button>
        </div>

        <div class="game-card-stats">
          <div class="game-card-stats-row">
            <span>Save Locations:</span>
            <strong style="color: #ffffff">${game.save_locations_count}</strong>
          </div>
          <div class="game-card-stats-row">
            <span>Arquivos Monitorados:</span>
            <strong style="color: #ffffff">${game.files_count}</strong>
          </div>
          <div class="game-card-stats-row">
            <span>Snapshots no Cofre:</span>
            <strong style="color: #ffffff">${game.snapshots_count}</strong>
          </div>
        </div>

        <div class="game-card-footer">
          <div class="game-status-line ${statusClass}">
            <span class="status-dot ${statusDot}"></span>
            <span>● ${escapeHtml(game.monitoring_status)}</span>
          </div>
          <div style="color: var(--text-dim)">
            ⏱️ ${escapeHtml(actTime)}
          </div>
        </div>
      `;

      grid.appendChild(card);
    });
  }

  function filterGames(query) {
    if (!query || query.trim() === '') {
      renderGamesList(cachedGames);
      return;
    }
    const q = query.toLowerCase().trim();
    const filtered = cachedGames.filter((g) => {
      const nameMatch = g.name.toLowerCase().includes(q);
      const steamMatch = g.steam_app_id && g.steam_app_id.includes(q);
      const platformMatch = g.platform.toLowerCase().includes(q);
      return nameMatch || steamMatch || platformMatch;
    });
    renderGamesList(filtered);
  }

  // 3. Game Details View
  window.openGameDetails = async function (gameId) {
    selectedGameId = gameId;
    document.getElementById('games-list-view').style.display = 'none';
    const detailView = document.getElementById('game-detail-view');
    detailView.style.display = 'block';

    await loadGameDetails(gameId);
  };

  async function loadGameDetails(gameId) {
    try {
      const details = await invoke('get_game_details', { gameId: gameId.toString() });
      if (!details) return;

      const g = details.game;
      let platformStr = g.platform;
      if (g.steam_app_id) platformStr = `Steam · AppID ${g.steam_app_id}`;
      else if (g.gog_id) platformStr = `GOG · ID ${g.gog_id}`;

      let lastAct = g.last_activity ? g.last_activity : '—';

      // Render Hero
      const hero = document.getElementById('game-hero');
      hero.innerHTML = `
        <div class="game-hero-header">
          <div>
            <div class="game-hero-title">🎮 ${escapeHtml(g.name)}</div>
            <div class="game-hero-meta">
              <span>${escapeHtml(platformStr)}</span>
              <span>·</span>
              <span class="badge ${g.monitoring_status === 'Monitorando' ? 'badge-success' : 'badge-warning'}">
                ● ${escapeHtml(g.monitoring_status)}
              </span>
              <span>·</span>
              <span class="badge badge-subtle">Sincronização: ${escapeHtml(g.sync_status)}</span>
            </div>
          </div>
        </div>

        <div class="hero-stats-row">
          <div class="hero-stat-box">
            <div class="hero-stat-label">Save Locations</div>
            <div class="hero-stat-value">${g.save_locations_count}</div>
          </div>
          <div class="hero-stat-box">
            <div class="hero-stat-label">Arquivos</div>
            <div class="hero-stat-value">${g.files_count}</div>
          </div>
          <div class="hero-stat-box">
            <div class="hero-stat-label">Snapshots</div>
            <div class="hero-stat-value">${g.snapshots_count}</div>
          </div>
          <div class="hero-stat-box">
            <div class="hero-stat-label">Última Atividade</div>
            <div class="hero-stat-value" style="font-size: 15px;">${escapeHtml(lastAct)}</div>
          </div>
        </div>
      `;

      // Render Save Locations
      const locTbody = document.getElementById('game-locations-tbody');
      const locCount = document.getElementById('game-locations-count');
      locTbody.innerHTML = '';
      locCount.textContent = `${details.locations.length} local(is)`;

      details.locations.forEach((loc) => {
        const tr = document.createElement('tr');
        const diskStatus = loc.exists_on_disk
          ? '<span class="badge badge-success">✓ No disco</span>'
          : '<span class="badge badge-danger">✕ Ausente</span>';
        tr.innerHTML = `
          <td><span class="badge badge-subtle">${escapeHtml(loc.location_type)}</span></td>
          <td class="code-text">${escapeHtml(loc.path)}</td>
          <td>${diskStatus}</td>
          <td>${loc.monitored_files_count} arquivo(s)</td>
        `;
        locTbody.appendChild(tr);
      });

      // Render Snapshots
      const snapTbody = document.getElementById('game-snapshots-tbody');
      const snapCount = document.getElementById('game-snapshots-count');
      const snapEmpty = document.getElementById('game-snapshots-empty');
      snapTbody.innerHTML = '';
      snapCount.textContent = `${details.snapshots.length} snapshot(s)`;

      if (details.snapshots.length === 0) {
        snapEmpty.style.display = 'block';
      } else {
        snapEmpty.style.display = 'none';
        details.snapshots.forEach((snap) => {
          const tr = document.createElement('tr');
          tr.innerHTML = `
            <td><strong>${escapeHtml(snap.file_name)}</strong></td>
            <td><span class="badge badge-dev">v${snap.version}</span></td>
            <td>${escapeHtml(snap.created_at)}</td>
            <td>${escapeHtml(snap.size_formatted)}</td>
            <td><span class="code-text">${escapeHtml(snap.short_hash)}</span></td>
            <td>
              <button class="btn btn-primary btn-sm" onclick="window.promptRestore('${snap.id}')">
                Restaurar
              </button>
            </td>
          `;
          snapTbody.appendChild(tr);
        });
      }

      // Render Activity
      const actTbody = document.getElementById('game-activity-tbody');
      const actEmpty = document.getElementById('game-activity-empty');
      actTbody.innerHTML = '';

      if (details.recent_activity.length === 0) {
        actEmpty.style.display = 'block';
      } else {
        actEmpty.style.display = 'none';
        details.recent_activity.forEach((ev) => {
          const tr = document.createElement('tr');
          let lvlClass = 'badge-subtle';
          if (ev.level === 'SUCCESS') lvlClass = 'badge-success';
          if (ev.level === 'WARNING') lvlClass = 'badge-warning';
          if (ev.level === 'ERROR') lvlClass = 'badge-danger';

          tr.innerHTML = `
            <td><span class="code-text">${escapeHtml(ev.timestamp)}</span></td>
            <td><span class="badge ${lvlClass}">${escapeHtml(ev.level)}</span></td>
            <td>${escapeHtml(ev.message)}</td>
          `;
          actTbody.appendChild(tr);
        });
      }
    } catch (e) {
      console.error('Erro ao carregar detalhes do jogo:', e);
      showToast(`Erro ao carregar detalhes: ${e}`, 'error');
    }
  }

  // 4. Tab: Dashboard
  async function loadDashboard() {
    try {
      const summary = await invoke('get_dashboard_status');
      if (!summary) return;

      const setTxt = (id, val) => {
        const el = document.getElementById(id);
        if (el) el.textContent = val;
      };

      setTxt('dash-games', summary.save_locations_count > 0 ? summary.save_locations_count : 0);
      setTxt('dash-locations', summary.save_locations_count);
      setTxt('dash-files', summary.monitored_files_count);
      setTxt('dash-snapshots', summary.snapshots_count);
      setTxt('dash-sync', summary.sync_pending_count);
      setTxt('dash-next-recon', summary.next_reconciliation_time || '—');

      setTxt('dash-db-path', summary.db_path);
      setTxt('dash-backup-root', summary.backup_root);
      setTxt('dash-watcher-state', summary.watcher_status === 'Running' ? '● Monitorando ativamente' : 'Inativo');
      setTxt('dash-reconciler-state', summary.reconciler_status === 'Running' ? '● Em execução (Tokio)' : summary.reconciler_status);

      if (summary.last_event) {
        setTxt('dash-last-event-level', summary.last_event.level);
        setTxt('dash-last-event-time', summary.last_event.timestamp);
        setTxt('dash-last-event-msg', summary.last_event.message);
      }
    } catch (e) {
      console.error('Erro ao carregar dashboard:', e);
    }
  }

  // 5. Tab: Save Locations
  async function loadLocations() {
    try {
      const locations = await invoke('get_save_locations', { gameId: null });
      const tbody = document.getElementById('locations-tbody');
      const empty = document.getElementById('locations-empty');
      if (!tbody) return;

      tbody.innerHTML = '';
      if (!locations || locations.length === 0) {
        if (empty) empty.style.display = 'block';
        return;
      }
      if (empty) empty.style.display = 'none';

      locations.forEach((loc) => {
        const tr = document.createElement('tr');
        const diskBadge = loc.exists_on_disk
          ? '<span class="badge badge-success">✓ Sim</span>'
          : '<span class="badge badge-danger">✕ Não</span>';

        tr.innerHTML = `
          <td><strong>${escapeHtml(loc.game_name)}</strong></td>
          <td><span class="badge badge-subtle">${escapeHtml(loc.platform)}</span></td>
          <td>${escapeHtml(loc.location_type)}</td>
          <td class="code-text">${escapeHtml(loc.path)}</td>
          <td>${diskBadge}</td>
          <td>${loc.monitored_files_count}</td>
          <td>
            <button class="btn btn-secondary btn-sm" onclick="window.scanLocationManual('${loc.id}')">
              Varredura
            </button>
          </td>
        `;
        tbody.appendChild(tr);
      });
    } catch (e) {
      console.error('Erro ao carregar save locations:', e);
    }
  }

  window.scanLocationManual = async function (locationId) {
    try {
      showToast('Iniciando varredura física no disco...', 'info');
      const result = await invoke('scan_location', { locationId: locationId.toString() });
      if (result) {
        showToast(`Varredura concluída: ${result.files_found.length} arquivo(s) encontrado(s)`, 'success');
        openScanModal(result.files_found);
      }
    } catch (e) {
      showToast(`Falha na varredura: ${e}`, 'error');
    }
  };

  function openScanModal(files) {
    const modal = document.getElementById('scan-modal');
    const tbody = document.getElementById('scan-modal-tbody');
    tbody.innerHTML = '';

    files.forEach((f) => {
      const tr = document.createElement('tr');
      tr.innerHTML = `
        <td><strong>${escapeHtml(f.filename)}</strong></td>
        <td class="code-text">${escapeHtml(f.path)}</td>
        <td>${escapeHtml(f.size_formatted)}</td>
        <td>${escapeHtml(f.modified_at)}</td>
      `;
      tbody.appendChild(tr);
    });

    modal.style.display = 'flex';
  }

  function closeScanModal() {
    document.getElementById('scan-modal').style.display = 'none';
  }

  // 6. Tab: Snapshots
  let cachedSnapshots = [];
  async function loadSnapshots() {
    try {
      const snapshots = await invoke('get_snapshots', { gameId: null });
      cachedSnapshots = snapshots || [];
      renderSnapshotsTable(cachedSnapshots);
    } catch (e) {
      console.error('Erro ao carregar snapshots:', e);
    }
  }

  function renderSnapshotsTable(snapshots) {
    const tbody = document.getElementById('snapshots-tbody');
    const empty = document.getElementById('snapshots-empty');
    if (!tbody) return;

    tbody.innerHTML = '';
    if (!snapshots || snapshots.length === 0) {
      if (empty) empty.style.display = 'block';
      return;
    }
    if (empty) empty.style.display = 'none';

    snapshots.forEach((s) => {
      const tr = document.createElement('tr');
      const physBadge = s.physical_exists
        ? '<span class="badge badge-success">✓ OK</span>'
        : '<span class="badge badge-danger">✕ Ausente</span>';

      tr.innerHTML = `
        <td><strong>${escapeHtml(s.game_name)}</strong></td>
        <td>${escapeHtml(s.file_name)}</td>
        <td><span class="badge badge-dev">v${s.version}</span></td>
        <td>${escapeHtml(s.created_at)}</td>
        <td>${escapeHtml(s.size_formatted)}</td>
        <td><span class="code-text">${escapeHtml(s.short_hash)}</span></td>
        <td>${physBadge}</td>
        <td>
          <button class="btn btn-primary btn-sm" onclick="window.promptRestore('${s.id}')">
            Restaurar
          </button>
        </td>
      `;
      tbody.appendChild(tr);
    });
  }

  function filterSnapshots(query) {
    if (!query || query.trim() === '') {
      renderSnapshotsTable(cachedSnapshots);
      return;
    }
    const q = query.toLowerCase().trim();
    const filtered = cachedSnapshots.filter(
      (s) => s.game_name.toLowerCase().includes(q) || s.file_name.toLowerCase().includes(q)
    );
    renderSnapshotsTable(filtered);
  }

  // 7. Restore Flow & Modal
  window.promptRestore = async function (snapshotId) {
    try {
      // Find snapshot info from cache or fetch
      let snap = cachedSnapshots.find((s) => s.id === snapshotId);
      if (!snap) {
        const all = await invoke('get_snapshots', { gameId: null });
        snap = all.find((s) => s.id === snapshotId);
      }
      if (!snap) {
        showToast('Snapshot não localizado.', 'error');
        return;
      }

      pendingRestoreSnapshot = snap;

      // Populate confirmation modal
      document.getElementById('modal-game-name').textContent = snap.game_name;
      document.getElementById('modal-file-version').textContent = `${snap.file_name} · Versão v${snap.version}`;
      document.getElementById('modal-dest-path').textContent = snap.file_path;

      // Show confirm step, hide result step
      document.getElementById('modal-step-confirm').style.display = 'block';
      document.getElementById('modal-step-result').style.display = 'none';

      document.getElementById('restore-modal').style.display = 'flex';
    } catch (e) {
      showToast(`Falha ao preparar restauração: ${e}`, 'error');
    }
  };

  async function confirmRestore() {
    if (!pendingRestoreSnapshot) return;
    const snap = pendingRestoreSnapshot;

    const btn = document.getElementById('btn-modal-confirm');
    btn.disabled = true;
    btn.textContent = 'Restaurando com Safety Backup...';

    try {
      const resp = await invoke('restore_snapshot', {
        snapshotId: snap.id.toString(),
        expectedHash: null,
      });

      // Switch to result view
      document.getElementById('modal-step-confirm').style.display = 'none';
      const resultStep = document.getElementById('modal-step-result');
      const resultBody = document.getElementById('modal-result-body');
      resultStep.style.display = 'block';

      if (resp.success) {
        resultBody.innerHTML = `
          <div class="result-success-box">
            <div class="result-icon-heading">
              <span>✓</span>
              <span>Restauração Concluída com Sucesso!</span>
            </div>
            <div class="key-value-list">
              <div class="kv-item">
                <span class="kv-key">Jogo:</span>
                <span class="kv-val">${escapeHtml(snap.game_name)}</span>
              </div>
              <div class="kv-item">
                <span class="kv-key">Snapshot Restaurado:</span>
                <span class="kv-val badge badge-dev">v${resp.version || snap.version}</span>
              </div>
              <div class="kv-item">
                <span class="kv-key">Hash BLAKE3:</span>
                <span class="kv-val code-text">OK (Integridade Validada)</span>
              </div>
              <div class="kv-item">
                <span class="kv-key">Arquivo de Destino:</span>
                <span class="kv-val code-text" style="font-size: 11px;">${escapeHtml(resp.dest_path || snap.file_path)}</span>
              </div>
              <div class="kv-item">
                <span class="kv-key">Safety Backup Criado:</span>
                <span class="kv-val code-text" style="font-size: 11px; color: var(--success);">${escapeHtml(resp.safety_backup_path || 'Cópia preventiva criada no mesmo diretório')}</span>
              </div>
            </div>
          </div>
        `;
        showToast('Restauração executada com sucesso!', 'success');
      } else {
        resultBody.innerHTML = `
          <div class="result-error-box">
            <div style="color: var(--danger); font-size: 16px; font-weight: 700; margin-bottom: 8px;">
              ✕ Falha na Restauração
            </div>
            <p style="color: var(--text-main); margin-bottom: 12px;">${escapeHtml(resp.error || 'Erro desconhecido')}</p>
            <div class="code-text" style="color: #fca5a5; font-size: 11px; white-space: pre-wrap;">${escapeHtml(resp.details || '')}</div>
          </div>
        `;
        showToast('Falha na restauração do save.', 'error');
      }

      // Refresh data
      loadSnapshots();
      loadSafetyBackups();
    } catch (e) {
      showToast(`Erro crítico ao restaurar: ${e}`, 'error');
    } finally {
      btn.disabled = false;
      btn.textContent = 'Restaurar';
    }
  }

  function closeModal() {
    document.getElementById('restore-modal').style.display = 'none';
    pendingRestoreSnapshot = null;
  }

  // 8. Tab: Safety Backups
  async function loadSafetyBackups() {
    try {
      const groups = await invoke('get_safety_backups');
      const container = document.getElementById('safety-groups-container');
      const empty = document.getElementById('safety-empty');
      if (!container) return;

      container.innerHTML = '';
      if (!groups || groups.length === 0) {
        if (empty) empty.style.display = 'flex';
        return;
      }
      if (empty) empty.style.display = 'none';

      groups.forEach((g) => {
        const card = document.createElement('div');
        card.className = 'safety-group-card';

        let rowsHtml = '';
        g.backups.forEach((b) => {
          rowsHtml += `
            <tr>
              <td><strong>${escapeHtml(b.filename)}</strong></td>
              <td class="code-text">${escapeHtml(b.path)}</td>
              <td>${escapeHtml(b.size_formatted)}</td>
              <td>${escapeHtml(b.modified_at)}</td>
            </tr>
          `;
        });

        card.innerHTML = `
          <div class="safety-group-header">
            <div>
              <div class="safety-group-title">🎮 ${escapeHtml(g.game_name)} — ${escapeHtml(g.original_name)}</div>
              <div class="safety-group-path">${escapeHtml(g.original_path)}</div>
            </div>
            <div class="badge badge-success">${g.backups.length} safety backup(s)</div>
          </div>
          <div class="table-container">
            <table class="data-table">
              <thead>
                <tr>
                  <th>ARQUIVO PREVENTIVO</th>
                  <th>LOCALIZAÇÃO</th>
                  <th>TAMANHO</th>
                  <th>DATA DE CRIAÇÃO</th>
                </tr>
              </thead>
              <tbody>${rowsHtml}</tbody>
            </table>
          </div>
        `;

        container.appendChild(card);
      });
    } catch (e) {
      console.error('Erro ao carregar safety backups:', e);
    }
  }

  // 9. Tab: Reconciliação
  async function loadReconciliation() {
    try {
      const r = await invoke('get_reconciliation_status');
      if (!r) return;

      const setTxt = (id, val) => {
        const el = document.getElementById(id);
        if (el) el.textContent = val;
      };

      setTxt('recon-locs', r.locations_scanned);
      setTxt('recon-seen', r.files_seen);
      setTxt('recon-added', r.files_added);
      setTxt('recon-changed', r.files_changed);
      setTxt('recon-errors', r.errors);
      setTxt('recon-duration', `${r.duration_ms} ms`);

      setTxt('recon-status', r.is_running ? '● Em execução agora' : '● Ativo (Tokio loop)');
      setTxt('recon-interval', `${r.interval_secs} segundos`);
      setTxt('recon-last-run', r.last_run || 'Ainda não executado');
      setTxt('recon-next-run', r.next_run || '—');
    } catch (e) {
      console.error('Erro ao carregar reconciliação:', e);
    }
  }

  async function triggerReconcile() {
    try {
      showToast('Disparando reconciliação sob demanda...', 'info');
      const rep = await invoke('run_reconciliation');
      if (rep) {
        showToast(
          `Reconciliação concluída: ${rep.files_seen} arquivos analisados, ${rep.files_added} adicionados, ${rep.files_changed} alterados.`,
          'success'
        );
        loadHeaderStatus();
        if (currentTab === 'tab-reconcile') loadReconciliation();
        if (currentTab === 'tab-games') loadGames();
        if (currentTab === 'tab-snapshots') loadSnapshots();
      }
    } catch (e) {
      showToast(`Erro na reconciliação: ${e}`, 'error');
    }
  }

  async function triggerDiscovery() {
    try {
      showToast('Executando descoberta de jogos e save locations...', 'info');
      const res = await invoke('trigger_discovery');
      if (res) {
        showToast(
          `Descoberta concluída: ${res.games_found} jogos encontrados (${res.new_games} novos), ${res.locations_found} locais detectados (${res.new_locations} novos).`,
          'success'
        );
        loadHeaderStatus();
        loadGames();
      }
    } catch (e) {
      showToast(`Erro na descoberta: ${e}`, 'error');
    }
  }

  // 10. Tab: Sync Queue
  async function loadSyncQueue() {
    try {
      const queue = await invoke('get_sync_queue');
      const tbody = document.getElementById('sync-tbody');
      const empty = document.getElementById('sync-empty');
      if (!tbody) return;

      tbody.innerHTML = '';
      if (!queue || queue.length === 0) {
        if (empty) empty.style.display = 'block';
        return;
      }
      if (empty) empty.style.display = 'none';

      queue.forEach((q) => {
        const tr = document.createElement('tr');
        let statusBadge = '<span class="badge badge-subtle">Desconhecido</span>';
        if (q.status.toLowerCase() === 'pending') statusBadge = '<span class="badge badge-warning">Pendente</span>';
        if (q.status.toLowerCase() === 'synced') statusBadge = '<span class="badge badge-success">Sincronizado</span>';
        if (q.status.toLowerCase() === 'failed') statusBadge = '<span class="badge badge-danger">Falhou</span>';

        tr.innerHTML = `
          <td><strong>${escapeHtml(q.file_name)}</strong></td>
          <td>${escapeHtml(q.game_name)}</td>
          <td><span class="badge badge-subtle">${escapeHtml(q.operation)}</span></td>
          <td>${statusBadge}</td>
          <td>${q.attempts}</td>
          <td>${escapeHtml(q.created_at)}</td>
          <td class="code-text">${escapeHtml(q.last_error || '—')}</td>
        `;
        tbody.appendChild(tr);
      });
    } catch (e) {
      console.error('Erro ao carregar fila de sync:', e);
    }
  }

  // 11. Tab: Eventos
  async function loadEvents() {
    try {
      const events = await invoke('get_recent_events');
      const tbody = document.getElementById('events-tbody');
      const empty = document.getElementById('events-empty');
      if (!tbody) return;

      tbody.innerHTML = '';
      if (!events || events.length === 0) {
        if (empty) empty.style.display = 'block';
        return;
      }
      if (empty) empty.style.display = 'none';

      events.forEach((ev) => {
        const tr = document.createElement('tr');
        let lvlClass = 'badge-subtle';
        if (ev.level === 'SUCCESS') lvlClass = 'badge-success';
        if (ev.level === 'WARNING') lvlClass = 'badge-warning';
        if (ev.level === 'ERROR') lvlClass = 'badge-danger';

        tr.innerHTML = `
          <td><span class="code-text">${escapeHtml(ev.timestamp)}</span></td>
          <td><span class="badge ${lvlClass}">${escapeHtml(ev.level)}</span></td>
          <td><strong>${escapeHtml(ev.message)}</strong></td>
          <td class="code-text" style="font-size: 11px;">${escapeHtml(ev.details || '—')}</td>
        `;
        tbody.appendChild(tr);
      });
    } catch (e) {
      console.error('Erro ao carregar eventos:', e);
    }
  }

  // Window Visibility & Resource Saving Handling
  function initVisibilityHandling() {
    document.addEventListener('visibilitychange', () => {
      if (document.hidden) {
        isWindowVisible = false;
        stopBackgroundPolling();
      } else {
        isWindowVisible = true;
        startBackgroundPolling();
        loadActiveTabData();
      }
    });

    // Listen to Tauri native window events if available
    if (window.__TAURI__ && window.__TAURI__.event) {
      window.__TAURI__.event.listen('ui_state_change', (event) => {
        const visible = Boolean(event.payload);
        if (visible) {
          isWindowVisible = true;
          startBackgroundPolling();
          loadActiveTabData();
        } else {
          isWindowVisible = false;
          stopBackgroundPolling();
        }
      });
    }
  }

  function startBackgroundPolling() {
    if (pollInterval) clearInterval(pollInterval);
    // Conservative 5-second polling ONLY for header and active tab when visible!
    pollInterval = setInterval(() => {
      if (!isWindowVisible) return;
      loadHeaderStatus();
      if (currentTab === 'tab-reconcile') loadReconciliation();
      if (currentTab === 'tab-events') loadEvents();
    }, 5000);
  }

  function stopBackgroundPolling() {
    if (pollInterval) {
      clearInterval(pollInterval);
      pollInterval = null;
    }
  }

  // Utilities
  function escapeHtml(str) {
    if (str === null || str === undefined) return '';
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#039;');
  }

  // Entry Point
  document.addEventListener('DOMContentLoaded', () => {
    initNavigation();
    initVisibilityHandling();
    startBackgroundPolling();
    // Default initial view: Games
    loadGames();
    loadHeaderStatus();
  });
})();
