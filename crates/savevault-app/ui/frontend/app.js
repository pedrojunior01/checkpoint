const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

const appWindow = getCurrentWindow();

// ── Utilitários ───────────────────────────────────────────────────────────────

function timeAgo(isoString) {
    if (!isoString) return "—";
    const now = new Date();
    const then = new Date(isoString);
    const diff = Math.floor((now - then) / 1000);
    if (diff < 60) return "agora mesmo";
    if (diff < 3600) return `há ${Math.floor(diff / 60)} min`;
    if (diff < 86400) return `há ${Math.floor(diff / 3600)}h`;
    return then.toLocaleDateString("pt-BR", { day: "2-digit", month: "short" });
}

function formatHour(isoString) {
    if (!isoString) return "—";
    const d = new Date(isoString);
    const today = new Date();
    const prefix = d.toDateString() === today.toDateString() ? "Hoje" : d.toLocaleDateString("pt-BR", { day: "2-digit", month: "short" });
    return `${prefix}, ${d.toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })}`;
}

function formatBytes(bytes) {
    if (!bytes || bytes === 0) return "0 B";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1048576).toFixed(1)} MB`;
}

// ── Controles da janela ───────────────────────────────────────────────────────

document.getElementById("titlebar-minimize")?.addEventListener("click", () => appWindow.minimize());
document.getElementById("titlebar-maximize")?.addEventListener("click", () => appWindow.toggleMaximize());
document.getElementById("titlebar-close")?.addEventListener("click", () => appWindow.close());

// ── Navegação de abas ─────────────────────────────────────────────────────────

const tabs = {
    "nav-inicio":       "tab-inicio",
    "nav-jogos":        "tab-jogos",
    "nav-atividade":    "tab-atividade",
    "nav-nuvem":        "tab-nuvem",
    "nav-config":       "tab-config",
};

function showTab(navId) {
    // Esconde todas as abas
    Object.values(tabs).forEach(tabId => {
        const el = document.getElementById(tabId);
        if (el) el.classList.add("hidden");
    });
    // Remove estado ativo de todos os links
    Object.keys(tabs).forEach(id => {
        const el = document.getElementById(id);
        if (el) el.classList.remove("active");
    });
    // Mostra a aba selecionada
    const tabId = tabs[navId];
    const tab = document.getElementById(tabId);
    if (tab) tab.classList.remove("hidden");
    // Ativa o link
    const nav = document.getElementById(navId);
    if (nav) nav.classList.add("active");
}

// Liga os cliques da sidebar
Object.keys(tabs).forEach(navId => {
    document.getElementById(navId)?.addEventListener("click", (e) => {
        e.preventDefault();
        showTab(navId);
        if (navId === "nav-jogos") loadJogos();
        if (navId === "nav-atividade") loadAtividade();
        if (navId === "nav-nuvem") loadNuvem();
        if (navId === "nav-config") loadConfig();
    });
});

// Link "Ver todos" na aba Início
document.getElementById("link-ver-todos")?.addEventListener("click", (e) => {
    e.preventDefault();
    showTab("nav-jogos");
    loadJogos();
});

// ── Aba: Início ───────────────────────────────────────────────────────────────

async function loadInicio() {
    try {
        const data = await invoke("get_dashboard_status");

        // KPIs
        const elGames = document.getElementById("kpi-games");
        const elSaves = document.getElementById("kpi-saves");
        const elTime = document.getElementById("kpi-time");
        if (elGames) elGames.textContent = data.save_locations_count ?? 0;
        if (elSaves) elSaves.textContent = data.snapshots_count ?? 0;
        if (elTime) elTime.textContent = formatHour(data.last_reconciliation_time);

        // Status banner
        const bannerText = document.getElementById("status-text");
        if (data.sync_pending_count > 0) {
            if (bannerText) bannerText.textContent = `${data.sync_pending_count} item(s) aguardando sincronização...`;
        } else {
            if (bannerText) bannerText.textContent = "Seus saves estão protegidos e sincronizados.";
        }
    } catch (e) {
        console.error("Erro ao carregar dashboard:", e);
    }

    // Jogos recentes (os primeiros da lista de locais)
    try {
        const locs = await invoke("get_locations");
        renderJogosRecentes(locs.slice(0, 4));
    } catch (e) {
        console.error("Erro ao carregar jogos recentes:", e);
    }
}

function renderJogosRecentes(locations) {
    const container = document.getElementById("jogos-recentes-list");
    if (!container) return;
    if (!locations || locations.length === 0) {
        container.innerHTML = `<p style="font-size:13px;color:#94a3b8;padding:8px 0;">Nenhum jogo detectado ainda.</p>`;
        return;
    }
    container.innerHTML = locations.map((loc, i) => {
        const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
        const gradClass = `grad-${i % 6}`;
        return `
        <div class="game-item">
            <div class="game-avatar ${gradClass}">
                ${initials}
            </div>
            <div class="game-info">
                <p class="game-name">${loc.game_name || "Jogo"}</p>
                <p class="game-subtext">${loc.files_count ?? 0} arquivo(s) · ${timeAgo(loc.last_seen_at)}</p>
            </div>
            <span class="badge-synced">Sincronizado</span>
        </div>`;
    }).join("");
}

// ── Aba: Jogos ────────────────────────────────────────────────────────────────

async function loadJogos() {
    const container = document.getElementById("jogos-list");
    if (!container) return;
    container.innerHTML = `<p style="font-size:13px;color:#94a3b8;">Carregando...</p>`;
    try {
        const locs = await invoke("get_locations");
        if (!locs || locs.length === 0) {
            container.innerHTML = `<p style="font-size:13px;color:#94a3b8;">Nenhum jogo detectado. Execute um scan.</p>`;
            return;
        }
        container.innerHTML = locs.map((loc, i) => {
            const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
            const gradClass = `grad-${i % 6}`;
            return `
            <div class="content-card" style="display:flex;align-items:center;gap:14px;margin-bottom:10px;padding:14px;">
                <div class="game-avatar ${gradClass}" style="width:44px;height:44px;font-size:14px;">
                    ${initials}
                </div>
                <div class="game-info">
                    <p class="game-name" style="font-size:14px;">${loc.game_name || "Jogo Desconhecido"}</p>
                    <p class="game-subtext" style="font-family:ui-monospace, monospace;color:#64748b;">${loc.path || ""}</p>
                    <p class="game-subtext">${loc.files_count ?? 0} arquivo(s) monitorado(s)</p>
                </div>
                <span class="badge-synced">Ativo</span>
            </div>`;
        }).join("");
    } catch (e) {
        container.innerHTML = `<p style="font-size:13px;color:#ef4444;">Erro: ${e}</p>`;
    }
}

// ── Aba: Atividade ────────────────────────────────────────────────────────────

async function loadAtividade() {
    const container = document.getElementById("atividade-list");
    if (!container) return;
    container.innerHTML = `<p style="font-size:13px;color:#94a3b8;">Carregando histórico...</p>`;
    try {
        const snapshots = await invoke("get_snapshots");
        if (!snapshots || snapshots.length === 0) {
            container.innerHTML = `<p style="font-size:13px;color:#94a3b8;">Nenhum snapshot encontrado ainda.</p>`;
            return;
        }
        container.innerHTML = snapshots.map(s => `
            <div class="activity-item">
                <div class="dot-indicator"></div>
                <div class="game-info">
                    <p class="game-name">${s.game_name || "Arquivo de Save"}</p>
                    <p class="game-subtext" style="font-family:ui-monospace, monospace;">${(s.blake3_hash || "").substring(0, 16)}…</p>
                    <p class="game-subtext">${s.file_size_bytes ? formatBytes(s.file_size_bytes) : ""} · ${timeAgo(s.captured_at)}</p>
                </div>
                <button
                    onclick="restoreSnapshot('${s.snapshot_id}', '${(s.game_name || "").replace(/'/g, "")}')"
                    class="btn-restore">
                    Restaurar
                </button>
            </div>`).join("");
    } catch (e) {
        container.innerHTML = `<p style="font-size:13px;color:#ef4444;">Erro: ${e}</p>`;
    }
}

async function restoreSnapshot(id, gameName) {
    if (!confirm(`Restaurar save de "${gameName}"?\n\nUm backup de segurança será criado antes da restauração.`)) return;
    try {
        const result = await invoke("restore_snapshot", { snapshotId: id });
        alert(`Restauração concluída!\nArquivo: ${result.restored_path || "OK"}`);
        loadAtividade();
    } catch (e) {
        alert(`Erro ao restaurar: ${e}`);
    }
}

// ── Aba: Nuvem ────────────────────────────────────────────────────────────────

async function loadNuvem() {
    const container = document.getElementById("nuvem-info");
    if (!container) return;
    try {
        const data = await invoke("get_dashboard_status");
        container.innerHTML = `
            <div class="content-card" style="display:flex;flex-direction:column;gap:12px;">
                <div style="display:flex;align-items:center;gap:12px;">
                    <div style="width:40px;height:40px;border-radius:10px;background:#eff6ff;display:flex;align-items:center;justify-content:center;font-size:20px;">☁️</div>
                    <div>
                        <p style="font-size:14px;font-weight:600;color:#0f172a;">Google Drive</p>
                        <p style="font-size:12px;color:#64748b;">${data.app_status || "Verificando..."}</p>
                    </div>
                </div>
                <div style="font-size:13px;color:#334155;border-top:1px solid #f1f5f9;padding-top:10px;line-height:1.6;">
                    <p>📦 Itens na fila de sync: <strong>${data.sync_pending_count ?? 0}</strong></p>
                    <p>🔄 Último sync: <strong>${formatHour(data.last_reconciliation_time)}</strong></p>
                </div>
            </div>`;
    } catch (e) {
        container.innerHTML = `<p style="font-size:13px;color:#ef4444;">Erro ao carregar status da nuvem: ${e}</p>`;
    }
}

// ── Aba: Configurações ────────────────────────────────────────────────────────

async function loadConfig() {
    const container = document.getElementById("config-info");
    if (!container) return;
    try {
        const data = await invoke("get_dashboard_status");
        container.innerHTML = `
            <div>
                <div class="config-card">
                    <p class="config-label">Banco de Dados</p>
                    <p class="config-value">${data.db_path || "—"}</p>
                </div>
                <div class="config-card">
                    <p class="config-label">Pasta de Backups</p>
                    <p class="config-value">${data.backup_root || "—"}</p>
                </div>
                <div class="config-card">
                    <p class="config-label">Intervalo de Reconciliação</p>
                    <p class="config-value" style="font-family:inherit;">${data.reconciliation_interval_secs ?? 30} segundos</p>
                </div>
                <div class="config-card">
                    <p class="config-label">Status do Watcher</p>
                    <p class="config-value" style="font-family:inherit;">${data.watcher_status || "—"}</p>
                </div>
            </div>`;
    } catch (e) {
        container.innerHTML = `<p style="font-size:13px;color:#ef4444;">Erro: ${e}</p>`;
    }
}

// ── Botão Reconciliar ─────────────────────────────────────────────────────────

document.getElementById("btn-reconcile")?.addEventListener("click", async () => {
    const btn = document.getElementById("btn-reconcile");
    if (btn) {
        btn.textContent = "Sincronizando...";
        btn.disabled = true;
    }
    try {
        const result = await invoke("trigger_reconcile");
        alert(`Reconciliação concluída: ${result}`);
        loadInicio();
    } catch (e) {
        alert(`Erro: ${e}`);
    } finally {
        if (btn) {
            btn.textContent = "Sincronizar Agora";
            btn.disabled = false;
        }
    }
});

// ── Init ──────────────────────────────────────────────────────────────────────

document.addEventListener("DOMContentLoaded", () => {
    showTab("nav-inicio");
    loadInicio();
    // Atualiza dashboard a cada 10s
    setInterval(loadInicio, 10000);
});
