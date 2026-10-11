//! Assets embutidos para a interface visual provisória (DEV-UI-01).

pub const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="pt-BR">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>checkpoint — DEV-UI-01</title>
  <style>
    :root {
      --bg-dark: #0f172a;
      --bg-card: #1e293b;
      --bg-card-hover: #334155;
      --border-color: #334155;
      --text-main: #f8fafc;
      --text-muted: #94a3b8;
      --primary: #3b82f6;
      --primary-hover: #2563eb;
      --success: #10b981;
      --success-bg: rgba(16, 185, 129, 0.15);
      --warning: #f59e0b;
      --warning-bg: rgba(245, 158, 11, 0.15);
      --danger: #ef4444;
      --danger-bg: rgba(239, 68, 68, 0.15);
      --info: #06b6d4;
      --info-bg: rgba(6, 182, 212, 0.15);
      --radius: 8px;
    }

    * { box-sizing: border-box; margin: 0; padding: 0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Oxygen, Ubuntu, Cantarell, sans-serif; }
    body { background-color: var(--bg-dark); color: var(--text-main); min-height: 100vh; display: flex; flex-direction: column; }

    /* Header */
    header { background: #090d16; border-bottom: 1px solid var(--border-color); padding: 12px 24px; display: flex; align-items: center; justify-content: space-between; position: sticky; top: 0; z-index: 100; }
    .brand { display: flex; align-items: center; gap: 12px; }
    .brand h1 { font-size: 1.25rem; font-weight: 700; letter-spacing: -0.5px; }
    .dev-badge { background: #312e81; color: #a5b4fc; font-size: 0.7rem; font-weight: 600; padding: 3px 8px; border-radius: 4px; text-transform: uppercase; border: 1px solid #4338ca; }
    .header-actions { display: flex; align-items: center; gap: 16px; }
    .status-pill { display: inline-flex; align-items: center; gap: 6px; font-size: 0.85rem; padding: 4px 10px; border-radius: 9999px; font-weight: 500; }
    .status-pill.online { background: var(--success-bg); color: #34d399; }
    .status-pill.warning { background: var(--warning-bg); color: #fbbf24; }
    .status-pill.error { background: var(--danger-bg); color: #f87171; }
    .status-dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
    .status-pill.online .status-dot { background: var(--success); }
    .status-pill.warning .status-dot { background: var(--warning); }
    .status-pill.error .status-dot { background: var(--danger); }

    /* Navigation */
    nav { background: #131d31; border-bottom: 1px solid var(--border-color); display: flex; padding: 0 24px; gap: 8px; overflow-x: auto; }
    .nav-btn { background: none; border: none; color: var(--text-muted); padding: 12px 16px; font-size: 0.9rem; font-weight: 500; cursor: pointer; border-bottom: 2px solid transparent; transition: all 0.2s; white-space: nowrap; }
    .nav-btn:hover { color: var(--text-main); }
    .nav-btn.active { color: var(--primary); border-bottom-color: var(--primary); }

    /* Main Container */
    main { flex: 1; padding: 24px; max-width: 1400px; margin: 0 auto; width: 100%; }
    .tab-content { display: none; }
    .tab-content.active { display: block; }

    /* Common Card Styles */
    .card { background: var(--bg-card); border: 1px solid var(--border-color); border-radius: var(--radius); padding: 20px; margin-bottom: 20px; }
    .card-title { font-size: 1.05rem; font-weight: 600; margin-bottom: 16px; display: flex; justify-content: space-between; align-items: center; }

    /* Buttons */
    .btn { background: var(--primary); color: white; border: none; padding: 8px 16px; border-radius: 6px; font-size: 0.85rem; font-weight: 500; cursor: pointer; transition: background 0.2s; display: inline-flex; align-items: center; gap: 6px; }
    .btn:hover { background: var(--primary-hover); }
    .btn-secondary { background: #334155; color: var(--text-main); }
    .btn-secondary:hover { background: #475569; }
    .btn-danger { background: var(--danger); }
    .btn-danger:hover { background: #dc2626; }
    .btn-success { background: var(--success); }
    .btn-success:hover { background: #059669; }
    .btn:disabled { opacity: 0.5; cursor: not-allowed; }

    /* Grid for Metrics */
    .metrics-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 16px; margin-bottom: 20px; }
    .metric-card { background: var(--bg-card); border: 1px solid var(--border-color); border-radius: var(--radius); padding: 16px; }
    .metric-label { font-size: 0.75rem; text-transform: uppercase; color: var(--text-muted); font-weight: 600; letter-spacing: 0.5px; }
    .metric-value { font-size: 1.75rem; font-weight: 700; margin-top: 6px; }
    .metric-sub { font-size: 0.8rem; color: var(--text-muted); margin-top: 4px; }

    /* Tables */
    .table-container { overflow-x: auto; background: var(--bg-card); border: 1px solid var(--border-color); border-radius: var(--radius); }
    table { width: 100%; border-collapse: collapse; text-align: left; font-size: 0.85rem; }
    th { background: #131d31; color: var(--text-muted); font-weight: 600; padding: 12px 16px; border-bottom: 1px solid var(--border-color); text-transform: uppercase; font-size: 0.75rem; }
    td { padding: 12px 16px; border-bottom: 1px solid var(--border-color); color: var(--text-main); vertical-align: middle; }
    tr:last-child td { border-bottom: none; }
    tr:hover td { background: rgba(255, 255, 255, 0.02); }
    code { font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace; background: #090d16; padding: 2px 6px; border-radius: 4px; font-size: 0.8rem; }

    /* Badges */
    .badge { display: inline-block; padding: 2px 8px; border-radius: 4px; font-size: 0.75rem; font-weight: 600; }
    .badge-info { background: var(--info-bg); color: var(--info); }
    .badge-success { background: var(--success-bg); color: var(--success); }
    .badge-warning { background: var(--warning-bg); color: var(--warning); }
    .badge-danger { background: var(--danger-bg); color: var(--danger); }
    .badge-muted { background: #334155; color: var(--text-muted); }

    /* Modals & Dialogs */
    .modal-backdrop { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.75); display: none; align-items: center; justify-content: center; z-index: 1000; padding: 16px; }
    .modal-backdrop.open { display: flex; }
    .modal { background: var(--bg-card); border: 1px solid var(--border-color); border-radius: 12px; width: 100%; max-width: 580px; box-shadow: 0 20px 25px -5px rgba(0, 0, 0, 0.5); overflow: hidden; }
    .modal-header { padding: 16px 20px; border-bottom: 1px solid var(--border-color); display: flex; justify-content: space-between; align-items: center; }
    .modal-header h3 { font-size: 1.1rem; font-weight: 600; }
    .modal-body { padding: 20px; font-size: 0.9rem; line-height: 1.5; }
    .modal-footer { padding: 16px 20px; border-top: 1px solid var(--border-color); display: flex; justify-content: flex-end; gap: 12px; background: #131d31; }
    .info-list { margin: 12px 0; display: flex; flex-direction: column; gap: 8px; }
    .info-item { display: flex; justify-content: space-between; border-bottom: 1px solid rgba(255, 255, 255, 0.05); padding-bottom: 6px; }
    .info-label { color: var(--text-muted); font-size: 0.85rem; }
    .info-val { font-weight: 500; font-family: monospace; font-size: 0.85rem; max-width: 340px; overflow: hidden; text-overflow: ellipsis; }

    /* Event Logs */
    .event-log { font-family: monospace; font-size: 0.85rem; display: flex; flex-direction: column; gap: 6px; max-height: 480px; overflow-y: auto; }
    .event-item { padding: 8px 12px; border-radius: 6px; background: #131d31; display: flex; gap: 12px; align-items: baseline; }
    .event-time { color: var(--text-muted); font-size: 0.75rem; min-width: 65px; }
    .event-level { font-weight: bold; font-size: 0.75rem; min-width: 65px; }
    .event-msg { flex: 1; word-break: break-word; }

    /* Toast Notification */
    #toast { position: fixed; bottom: 24px; right: 24px; background: var(--bg-card); border: 1px solid var(--border-color); border-radius: 8px; padding: 16px; box-shadow: 0 10px 15px -3px rgba(0, 0, 0, 0.5); z-index: 2000; transform: translateY(150%); transition: transform 0.3s ease; max-width: 400px; }
    #toast.show { transform: translateY(0); }
    #toast.success { border-color: var(--success); }
    #toast.error { border-color: var(--danger); }

    /* Loading Spinner */
    .spinner { display: inline-block; width: 14px; height: 14px; border: 2px solid rgba(255,255,255,0.3); border-radius: 50%; border-top-color: white; animation: spin 0.8s linear infinite; }
    @keyframes spin { to { transform: rotate(360deg); } }
  </style>
</head>
<body>
  <!-- Header -->
  <header>
    <div class="brand">
      <h1>checkpoint</h1>
      <span class="dev-badge">DEV-UI-01</span>
    </div>
    <div class="header-actions">
      <div id="app-status-badge" class="status-pill online">
        <span class="status-dot"></span>
        <span id="app-status-text">Online</span>
      </div>
      <button class="btn btn-secondary" id="btn-refresh" onclick="refreshCurrentTab()">
        <span>Atualizar</span>
      </button>
    </div>
  </header>

  <!-- Navigation -->
  <nav>
    <button class="nav-btn active" onclick="switchTab('dashboard')">📊 Dashboard</button>
    <button class="nav-btn" onclick="switchTab('locations')">📁 Save Locations</button>
    <button class="nav-btn" onclick="switchTab('snapshots')">💾 Snapshots</button>
    <button class="nav-btn" onclick="switchTab('safety')">🛡️ Safety Backups</button>
    <button class="nav-btn" onclick="switchTab('reconciliation')">🔄 Reconciliação</button>
    <button class="nav-btn" onclick="switchTab('events')">📜 Eventos & Log</button>
  </nav>

  <!-- Main Content -->
  <main>
    <!-- 1. DASHBOARD -->
    <div id="tab-dashboard" class="tab-content active">
      <div class="metrics-grid">
        <div class="metric-card">
          <div class="metric-label">Status do Sistema</div>
          <div class="metric-value" id="dash-app-status">Online</div>
          <div class="metric-sub" id="dash-db-status">Banco: Conectado</div>
        </div>
        <div class="metric-card">
          <div class="metric-label">Save Locations</div>
          <div class="metric-value" id="dash-locations-count">0</div>
          <div class="metric-sub" id="dash-files-count">0 arquivos monitorados</div>
        </div>
        <div class="metric-card">
          <div class="metric-label">Snapshots no Cofre</div>
          <div class="metric-value" id="dash-snapshots-count">0</div>
          <div class="metric-sub" id="dash-sync-count">0 pendentes de sync</div>
        </div>
        <div class="metric-card">
          <div class="metric-label">Watcher & Reconciler</div>
          <div class="metric-value" id="dash-watcher-status" style="font-size: 1.25rem; margin-top: 10px;">Watcher: Running</div>
          <div class="metric-sub" id="dash-reconciler-status">Reconciler: Running</div>
        </div>
      </div>

      <div class="card">
        <div class="card-title">
          <span>Ciclo de Reconciliação Periódica</span>
          <button class="btn btn-secondary" onclick="triggerReconcileNow(this)">🔄 Reconcile Now</button>
        </div>
        <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr)); gap: 12px; font-size: 0.9rem;">
          <div><strong>Intervalo:</strong> <span id="dash-recon-interval">30s</span></div>
          <div><strong>Último ciclo:</strong> <span id="dash-last-recon">Aguardando...</span></div>
          <div><strong>Próximo ciclo:</strong> <span id="dash-next-recon">Calculando...</span></div>
        </div>
      </div>

      <div class="card">
        <div class="card-title">Caminhos e Armazenamento</div>
        <div class="info-list">
          <div class="info-item">
            <span class="info-label">Banco SQLite</span>
            <span class="info-val" id="dash-db-path">-</span>
          </div>
          <div class="info-item">
            <span class="info-label">Cofre de Backups</span>
            <span class="info-val" id="dash-backup-root">-</span>
          </div>
        </div>
      </div>

      <div class="card" id="dash-error-card" style="display: none; border-color: var(--danger);">
        <div class="card-title" style="color: var(--danger);">Último Erro Registrado</div>
        <div id="dash-last-error" style="font-family: monospace; font-size: 0.85rem; color: #fca5a5;"></div>
      </div>
    </div>

    <!-- 2. SAVE LOCATIONS -->
    <div id="tab-locations" class="tab-content">
      <div class="card">
        <div class="card-title">
          <span>Locais de Save Monitorados</span>
        </div>
        <div class="table-container">
          <table id="locations-table">
            <thead>
              <tr>
                <th>Jogo</th>
                <th>Plataforma</th>
                <th>Tipo</th>
                <th>Caminho</th>
                <th>Status Disco</th>
                <th>Arquivos</th>
                <th>Ações</th>
              </tr>
            </thead>
            <tbody id="locations-tbody">
              <tr><td colspan="7" style="text-align: center; color: var(--text-muted);">Carregando locais...</td></tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>

    <!-- 3. SNAPSHOTS -->
    <div id="tab-snapshots" class="tab-content">
      <div class="card">
        <div class="card-title">
          <span>Histórico de Snapshots</span>
        </div>
        <div class="table-container">
          <table id="snapshots-table">
            <thead>
              <tr>
                <th>Snapshot ID</th>
                <th>Jogo</th>
                <th>Arquivo</th>
                <th>Versão</th>
                <th>Tamanho</th>
                <th>Hash BLAKE3</th>
                <th>Criado em</th>
                <th>Cofre</th>
                <th>Ação</th>
              </tr>
            </thead>
            <tbody id="snapshots-tbody">
              <tr><td colspan="9" style="text-align: center; color: var(--text-muted);">Carregando snapshots...</td></tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>

    <!-- 4. SAFETY BACKUPS -->
    <div id="tab-safety" class="tab-content">
      <div class="card">
        <div class="card-title">
          <span>Safety Backups de Proteção</span>
        </div>
        <p style="font-size: 0.85rem; color: var(--text-muted); margin-bottom: 16px;">
          Backups automáticos criados no mesmo diretório do arquivo original imediatamente antes de qualquer substituição de restore.
        </p>
        <div id="safety-groups-container">
          <!-- Grupos gerados dinamicamente -->
        </div>
      </div>
    </div>

    <!-- 5. RECONCILIAÇÃO -->
    <div id="tab-reconciliation" class="tab-content">
      <div class="card">
        <div class="card-title">
          <span>Status do Reconciliador Periódico</span>
          <button class="btn btn-secondary" onclick="triggerReconcileNow(this)">🔄 Reconcile Now</button>
        </div>
        <div class="metrics-grid">
          <div class="metric-card">
            <div class="metric-label">Status</div>
            <div class="metric-value" id="recon-status">Idle</div>
            <div class="metric-sub" id="recon-interval">Intervalo: 30s</div>
          </div>
          <div class="metric-card">
            <div class="metric-label">Locais / Arquivos Vistos</div>
            <div class="metric-value" id="recon-files-seen">0 / 0</div>
            <div class="metric-sub" id="recon-locations-scanned">0 locais escaneados</div>
          </div>
          <div class="metric-card">
            <div class="metric-label">Deltas (+ / ~ / -)</div>
            <div class="metric-value" id="recon-deltas">+0 / ~0 / -0</div>
            <div class="metric-sub" id="recon-errors">0 erros</div>
          </div>
          <div class="metric-card">
            <div class="metric-label">Duração do Último Ciclo</div>
            <div class="metric-value" id="recon-duration">0 ms</div>
            <div class="metric-sub" id="recon-last-time">-</div>
          </div>
        </div>
      </div>
    </div>

    <!-- 6. EVENTOS & LOG -->
    <div id="tab-events" class="tab-content">
      <div class="card">
        <div class="card-title">
          <span>Eventos e Diagnóstico do Sistema</span>
          <button class="btn btn-secondary" onclick="loadEvents()">Atualizar Log</button>
        </div>
        <div class="event-log" id="events-log-container">
          <!-- Eventos gerados dinamicamente -->
        </div>
      </div>
    </div>
  </main>

  <!-- MODAL: Confirmação de Restore -->
  <div class="modal-backdrop" id="restore-confirm-modal">
    <div class="modal">
      <div class="modal-header">
        <h3>Restaurar snapshot?</h3>
        <button class="btn btn-secondary" style="padding: 4px 8px;" onclick="closeRestoreModal()">✕</button>
      </div>
      <div class="modal-body">
        <div class="info-list">
          <div class="info-item">
            <span class="info-label">Arquivo:</span>
            <span class="info-val" id="modal-restore-file">-</span>
          </div>
          <div class="info-item">
            <span class="info-label">Versão:</span>
            <span class="info-val" id="modal-restore-version">-</span>
          </div>
          <div class="info-item">
            <span class="info-label">Tamanho:</span>
            <span class="info-val" id="modal-restore-size">-</span>
          </div>
          <div class="info-item">
            <span class="info-label">Hash BLAKE3:</span>
            <span class="info-val" id="modal-restore-hash">-</span>
          </div>
          <div class="info-item">
            <span class="info-label">Destino:</span>
            <span class="info-val" id="modal-restore-dest">-</span>
          </div>
        </div>
        <div style="background: var(--warning-bg); border-left: 3px solid var(--warning); padding: 10px; border-radius: 4px; font-size: 0.8rem; color: #fde68a; margin-top: 16px;">
          ⚠️ <strong>Proteção Ativa:</strong> Um <em>Safety Backup</em> será criado automaticamente antes da substituição caso o arquivo atual exista.
        </div>
      </div>
      <div class="modal-footer">
        <button class="btn btn-secondary" onclick="closeRestoreModal()">Cancelar</button>
        <button class="btn btn-danger" id="modal-btn-confirm-restore" onclick="confirmExecuteRestore()">Restaurar</button>
      </div>
    </div>
  </div>

  <!-- MODAL: Resultado do Restore -->
  <div class="modal-backdrop" id="restore-result-modal">
    <div class="modal">
      <div class="modal-header">
        <h3 id="result-modal-title">Resultado da Restauração</h3>
        <button class="btn btn-secondary" style="padding: 4px 8px;" onclick="closeResultModal()">✕</button>
      </div>
      <div class="modal-body" id="result-modal-body">
        <!-- Preenchido via JS -->
      </div>
      <div class="modal-footer">
        <button class="btn btn-secondary" onclick="closeResultModal()">Fechar</button>
      </div>
    </div>
  </div>

  <!-- MODAL: Scan Result -->
  <div class="modal-backdrop" id="scan-result-modal">
    <div class="modal" style="max-width: 700px;">
      <div class="modal-header">
        <h3>Arquivos Encontrados no Scan</h3>
        <button class="btn btn-secondary" style="padding: 4px 8px;" onclick="closeScanModal()">✕</button>
      </div>
      <div class="modal-body">
        <div id="scan-results-container" style="max-height: 400px; overflow-y: auto;"></div>
      </div>
      <div class="modal-footer">
        <button class="btn btn-secondary" onclick="closeScanModal()">Fechar</button>
      </div>
    </div>
  </div>

  <!-- Toast -->
  <div id="toast"></div>

  <script>
    let activeTab = 'dashboard';
    let currentPendingRestoreSnapshot = null;
    let autoRefreshTimer = null;

    function switchTab(tabId) {
      activeTab = tabId;
      document.querySelectorAll('.nav-btn').forEach(btn => btn.classList.remove('active'));
      document.querySelectorAll('.tab-content').forEach(tab => tab.classList.remove('active'));

      const targetBtn = Array.from(document.querySelectorAll('.nav-btn')).find(b => b.getAttribute('onclick')?.includes(tabId));
      if (targetBtn) targetBtn.classList.add('active');

      const targetTab = document.getElementById('tab-' + tabId);
      if (targetTab) targetTab.classList.add('active');

      refreshCurrentTab();
    }

    function refreshCurrentTab() {
      switch (activeTab) {
        case 'dashboard': loadDashboard(); break;
        case 'locations': loadLocations(); break;
        case 'snapshots': loadSnapshots(); break;
        case 'safety': loadSafetyBackups(); break;
        case 'reconciliation': loadReconciliation(); break;
        case 'events': loadEvents(); break;
      }
    }

    // Auto-refresh a cada 4 segundos
    setInterval(() => {
      if (!document.querySelector('.modal-backdrop.open')) {
        refreshCurrentTab();
      }
    }, 4000);

    // Initial Load
    window.addEventListener('DOMContentLoaded', () => {
      loadDashboard();
    });

    function showToast(msg, isError = false) {
      const toast = document.getElementById('toast');
      toast.textContent = msg;
      toast.className = isError ? 'show error' : 'show success';
      setTimeout(() => { toast.classList.remove('show'); }, 3500);
    }

    // ==========================================
    // 1. DASHBOARD
    // ==========================================
    async function loadDashboard() {
      try {
        const res = await fetch('/api/dashboard');
        const data = await res.json();

        document.getElementById('dash-app-status').textContent = data.app_status;
        document.getElementById('dash-db-status').textContent = `Banco: ${data.db_status}`;
        document.getElementById('dash-locations-count').textContent = data.save_locations_count;
        document.getElementById('dash-files-count').textContent = `${data.monitored_files_count} arquivos monitorados`;
        document.getElementById('dash-snapshots-count').textContent = data.snapshots_count;
        document.getElementById('dash-sync-count').textContent = `${data.sync_pending_count} pendentes de sync`;
        document.getElementById('dash-watcher-status').textContent = `Watcher: ${data.watcher_status}`;
        document.getElementById('dash-reconciler-status').textContent = `Reconciler: ${data.reconciler_status}`;
        document.getElementById('dash-recon-interval').textContent = `${data.reconciliation_interval_secs}s`;
        document.getElementById('dash-last-recon').textContent = data.last_reconciliation_time || 'Nenhum ciclo ainda';
        document.getElementById('dash-next-recon').textContent = data.next_reconciliation_time || 'Em breve';
        document.getElementById('dash-db-path').textContent = data.db_path;
        document.getElementById('dash-backup-root').textContent = data.backup_root;

        const badge = document.getElementById('app-status-badge');
        const badgeText = document.getElementById('app-status-text');
        badgeText.textContent = data.app_status;
        badge.className = `status-pill ${data.app_status.toLowerCase()}`;

        const errCard = document.getElementById('dash-error-card');
        if (data.last_error) {
          errCard.style.display = 'block';
          document.getElementById('dash-last-error').textContent = data.last_error;
        } else {
          errCard.style.display = 'none';
        }
      } catch (e) {
        console.error("Erro carregando dashboard:", e);
      }
    }

    // ==========================================
    // 2. LOCATIONS
    // ==========================================
    async function loadLocations() {
      try {
        const res = await fetch('/api/locations');
        const locations = await res.json();
        const tbody = document.getElementById('locations-tbody');
        tbody.innerHTML = '';

        if (locations.length === 0) {
          tbody.innerHTML = '<tr><td colspan="7" style="text-align: center; color: var(--text-muted);">Nenhum local cadastrado</td></tr>';
          return;
        }

        for (const loc of locations) {
          const tr = document.createElement('tr');
          const diskBadge = loc.exists_on_disk
            ? '<span class="badge badge-success">Presente</span>'
            : '<span class="badge badge-danger">Ausente</span>';

          tr.innerHTML = `
            <td><strong>${escapeHtml(loc.game_name)}</strong></td>
            <td><span class="badge badge-muted">${escapeHtml(loc.platform)}</span></td>
            <td>${escapeHtml(loc.location_type)}</td>
            <td><code title="${escapeHtml(loc.path)}">${escapeHtml(loc.path)}</code></td>
            <td>${diskBadge}</td>
            <td>${loc.monitored_files_count}</td>
            <td>
              <button class="btn btn-secondary" style="padding: 4px 10px; font-size: 0.75rem;" onclick="scanLocation('${loc.id}', this)">
                Scan
              </button>
            </td>
          `;
          tbody.appendChild(tr);
        }
      } catch (e) {
        console.error("Erro carregando locations:", e);
      }
    }

    async function scanLocation(locId, btn) {
      const origText = btn.innerHTML;
      btn.innerHTML = '<span class="spinner"></span>';
      btn.disabled = true;

      try {
        const res = await fetch(`/api/locations/scan?id=${encodeURIComponent(locId)}`, { method: 'POST' });
        const data = await res.json();

        let html = '';
        if (!data.files_found || data.files_found.length === 0) {
          html = '<p style="color: var(--text-muted); text-align: center; padding: 20px;">Nenhum arquivo encontrado no local de save informado.</p>';
        } else {
          html = '<table style="width: 100%; border-collapse: collapse; font-size: 0.85rem;"><thead><tr style="border-bottom: 1px solid var(--border-color);"><th style="padding: 8px;">Arquivo</th><th style="padding: 8px;">Tamanho</th><th style="padding: 8px;">Modificado</th></tr></thead><tbody>';
          for (const f of data.files_found) {
            html += `<tr style="border-bottom: 1px solid rgba(255,255,255,0.05);"><td style="padding: 8px;"><code>${escapeHtml(f.filename)}</code><div style="font-size: 0.75rem; color: var(--text-muted);">${escapeHtml(f.path)}</div></td><td style="padding: 8px;">${f.size_formatted}</td><td style="padding: 8px; font-family: monospace;">${f.modified_at}</td></tr>`;
          }
          html += '</tbody></table>';
        }
        document.getElementById('scan-results-container').innerHTML = html;
        document.getElementById('scan-result-modal').classList.add('open');
      } catch (e) {
        showToast("Erro no scan do local: " + e, true);
      } finally {
        btn.innerHTML = origText;
        btn.disabled = false;
      }
    }

    function closeScanModal() {
      document.getElementById('scan-result-modal').classList.remove('open');
    }

    // ==========================================
    // 3. SNAPSHOTS
    // ==========================================
    let cachedSnapshots = [];
    async function loadSnapshots() {
      try {
        const res = await fetch('/api/snapshots');
        cachedSnapshots = await res.json();
        const tbody = document.getElementById('snapshots-tbody');
        tbody.innerHTML = '';

        if (cachedSnapshots.length === 0) {
          tbody.innerHTML = '<tr><td colspan="9" style="text-align: center; color: var(--text-muted);">Nenhum snapshot encontrado no cofre</td></tr>';
          return;
        }

        for (const snap of cachedSnapshots) {
          const tr = document.createElement('tr');
          const physicalBadge = snap.physical_exists
            ? '<span class="badge badge-success">OK</span>'
            : '<span class="badge badge-danger">Ausente</span>';

          tr.innerHTML = `
            <td><code>${snap.id.substring(0, 8)}...</code></td>
            <td><strong>${escapeHtml(snap.game_name)}</strong></td>
            <td><code>${escapeHtml(snap.file_name)}</code></td>
            <td><span class="badge badge-info">v${snap.version}</span></td>
            <td>${snap.size_formatted}</td>
            <td title="${snap.hash}"><code>${snap.short_hash}</code></td>
            <td style="font-size: 0.8rem; color: var(--text-muted);">${snap.created_at.substring(0, 19)}</td>
            <td>${physicalBadge}</td>
            <td>
              <button class="btn btn-secondary" style="padding: 4px 10px; font-size: 0.75rem;" onclick="promptRestore('${snap.id}')">
                Restore
              </button>
            </td>
          `;
          tbody.appendChild(tr);
        }
      } catch (e) {
        console.error("Erro carregando snapshots:", e);
      }
    }

    function promptRestore(snapshotId) {
      const snap = cachedSnapshots.find(s => s.id === snapshotId);
      if (!snap) return;

      currentPendingRestoreSnapshot = snap;
      document.getElementById('modal-restore-file').textContent = snap.file_name;
      document.getElementById('modal-restore-version').textContent = `v${snap.version}`;
      document.getElementById('modal-restore-size').textContent = snap.size_formatted;
      document.getElementById('modal-restore-hash').textContent = snap.hash;
      document.getElementById('modal-restore-dest').textContent = snap.file_path;

      document.getElementById('restore-confirm-modal').classList.add('open');
    }

    function closeRestoreModal() {
      document.getElementById('restore-confirm-modal').classList.remove('open');
      currentPendingRestoreSnapshot = null;
    }

    async function confirmExecuteRestore() {
      if (!currentPendingRestoreSnapshot) return;
      const snap = currentPendingRestoreSnapshot;
      const btn = document.getElementById('modal-btn-confirm-restore');
      btn.innerHTML = '<span class="spinner"></span> Restaurando...';
      btn.disabled = true;

      try {
        const res = await fetch('/api/restore', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            snapshot_id: snap.id,
            expected_hash: snap.hash
          })
        });
        const rep = await res.json();
        closeRestoreModal();

        const resultModal = document.getElementById('restore-result-modal');
        const title = document.getElementById('result-modal-title');
        const body = document.getElementById('result-modal-body');

        if (rep.success) {
          title.innerHTML = '<span style="color: var(--success);">✓ Restauração concluída</span>';
          body.innerHTML = `
            <div class="info-list">
              <div class="info-item">
                <span class="info-label">Snapshot:</span>
                <span class="info-val">${rep.snapshot_id} (v${rep.version})</span>
              </div>
              <div class="info-item">
                <span class="info-label">Destino:</span>
                <span class="info-val">${rep.dest_path}</span>
              </div>
              <div class="info-item">
                <span class="info-label">Safety Backup:</span>
                <span class="info-val">${rep.safety_backup_path || '(nenhum arquivo pré-existente)'}</span>
              </div>
              <div class="info-item">
                <span class="info-label">Hash validado:</span>
                <span class="info-val">${rep.restored_hash}</span>
              </div>
              <div class="info-item">
                <span class="info-label">Bytes restaurados:</span>
                <span class="info-val">${rep.bytes_restored} B</span>
              </div>
            </div>
          `;
          showToast("Restauração concluída com sucesso!");
        } else {
          title.innerHTML = '<span style="color: var(--danger);">✕ Restauração falhou</span>';
          body.innerHTML = `
            <div style="margin-bottom: 12px; color: #fca5a5; font-weight: 500;">
              Motivo: ${escapeHtml(rep.error || 'Erro desconhecido')}
            </div>
            <div style="background: #090d16; padding: 10px; border-radius: 6px; font-family: monospace; font-size: 0.8rem; white-space: pre-wrap; margin-bottom: 12px; color: var(--text-muted);">
              ${escapeHtml(rep.details || '')}
            </div>
            <div style="color: var(--success); font-size: 0.85rem;">
              🛡️ O arquivo original foi preservado.
            </div>
          `;
          showToast("Falha na restauração", true);
        }

        resultModal.classList.add('open');
        refreshCurrentTab();
      } catch (e) {
        showToast("Erro durante requisição de restore: " + e, true);
      } finally {
        btn.innerHTML = 'Restaurar';
        btn.disabled = false;
      }
    }

    function closeResultModal() {
      document.getElementById('restore-result-modal').classList.remove('open');
    }

    // ==========================================
    // 4. SAFETY BACKUPS
    // ==========================================
    async function loadSafetyBackups() {
      try {
        const res = await fetch('/api/safety-backups');
        const groups = await res.json();
        const container = document.getElementById('safety-groups-container');
        container.innerHTML = '';

        if (groups.length === 0) {
          container.innerHTML = '<p style="color: var(--text-muted); padding: 20px; text-align: center;">Nenhum Safety Backup encontrado no disco.</p>';
          return;
        }

        for (const g of groups) {
          const groupCard = document.createElement('div');
          groupCard.className = 'card';
          groupCard.style.background = '#131d31';
          groupCard.style.padding = '14px';
          groupCard.style.marginBottom = '14px';

          let listHtml = '';
          for (let i = 0; i < g.backups.length; i++) {
            const b = g.backups[i];
            const isLast = i === g.backups.length - 1;
            const prefix = isLast ? '└── ' : '├── ';
            listHtml += `
              <div style="display: flex; justify-content: space-between; align-items: center; padding: 4px 0; font-family: monospace; font-size: 0.82rem;">
                <div>
                  <span style="color: var(--text-muted);">${prefix}</span>
                  <code>${escapeHtml(b.filename)}</code>
                </div>
                <div style="color: var(--text-muted); font-size: 0.75rem;">
                  <span>${b.size_formatted}</span> &bull; <span>${b.modified_at}</span>
                </div>
              </div>
            `;
          }

          groupCard.innerHTML = `
            <div style="font-weight: 600; margin-bottom: 8px; display: flex; justify-content: space-between;">
              <span>🎮 ${escapeHtml(g.game_name)} &mdash; <code>${escapeHtml(g.original_name)}</code></span>
              <span class="badge badge-info">${g.backups.length} backup(s)</span>
            </div>
            <div style="font-size: 0.75rem; color: var(--text-muted); margin-bottom: 8px;">${escapeHtml(g.original_path)}</div>
            <div style="margin-left: 8px; border-left: 1px dashed var(--border-color); padding-left: 10px;">
              ${listHtml}
            </div>
          `;
          container.appendChild(groupCard);
        }
      } catch (e) {
        console.error("Erro carregando safety backups:", e);
      }
    }

    // ==========================================
    // 5. RECONCILIAÇÃO
    // ==========================================
    async function loadReconciliation() {
      try {
        const res = await fetch('/api/reconciliation');
        const data = await res.json();

        document.getElementById('recon-status').textContent = data.status;
        document.getElementById('recon-interval').textContent = `Intervalo: ${data.interval_secs}s`;
        document.getElementById('recon-files-seen').textContent = `${data.locations_scanned} / ${data.files_seen}`;
        document.getElementById('recon-locations-scanned').textContent = `${data.locations_scanned} locais escaneados`;
        document.getElementById('recon-deltas').textContent = `+${data.files_added} / ~${data.files_changed} / -${data.files_removed}`;
        document.getElementById('recon-errors').textContent = `${data.errors} erro(s)`;
        document.getElementById('recon-duration').textContent = `${data.duration_ms} ms`;
        document.getElementById('recon-last-time').textContent = data.last_run ? `Última execução: ${data.last_run}` : 'Nenhum ciclo executado ainda';
      } catch (e) {
        console.error("Erro carregando reconciliação:", e);
      }
    }

    async function triggerReconcileNow(btn) {
      const origText = btn.innerHTML;
      btn.innerHTML = '<span class="spinner"></span> Reconciliando...';
      btn.disabled = true;

      try {
        const res = await fetch('/api/reconcile-now', { method: 'POST' });
        const report = await res.json();
        showToast(`Reconciliação concluída: ${report.files_seen} arquivos vistos em ${report.duration_ms || 0}ms`);
        refreshCurrentTab();
      } catch (e) {
        showToast("Erro ao acionar reconciliação: " + e, true);
      } finally {
        btn.innerHTML = origText;
        btn.disabled = false;
      }
    }

    // ==========================================
    // 6. EVENTOS & LOG
    // ==========================================
    async function loadEvents() {
      try {
        const res = await fetch('/api/events');
        const events = await res.json();
        const container = document.getElementById('events-log-container');
        container.innerHTML = '';

        if (events.length === 0) {
          container.innerHTML = '<p style="color: var(--text-muted); padding: 12px;">Nenhum evento registrado ainda.</p>';
          return;
        }

        for (const ev of events) {
          const div = document.createElement('div');
          div.className = 'event-item';

          let levelClass = 'badge-info';
          if (ev.level === 'SUCCESS') levelClass = 'badge-success';
          if (ev.level === 'WARNING') levelClass = 'badge-warning';
          if (ev.level === 'ERROR') levelClass = 'badge-danger';

          div.innerHTML = `
            <span class="event-time">${ev.timestamp}</span>
            <span class="badge ${levelClass}">${ev.level}</span>
            <div class="event-msg">
              <div>${escapeHtml(ev.message)}</div>
              ${ev.details ? `<div style="font-size: 0.75rem; color: var(--text-muted); margin-top: 4px;">${escapeHtml(ev.details)}</div>` : ''}
            </div>
          `;
          container.appendChild(div);
        }
      } catch (e) {
        console.error("Erro carregando eventos:", e);
      }
    }

    function escapeHtml(str) {
      if (!str) return '';
      return String(str)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#039;');
    }
  </script>
</body>
</html>
"#;
