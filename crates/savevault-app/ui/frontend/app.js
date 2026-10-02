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

function platformBadge(platform) {
    const icons = {
        "Steam": "🎮",
        "GOG": "🟣",
        "Dolphin": "🐬",
        "PCSX2": "💿",
        "Epic": "⚡",
    };
    return icons[platform] || "🎲";
}

// Cores de gradiente para avatares dos jogos (cicla baseado no índice)
const gradients = [
    "from-emerald-400 to-teal-600",
    "from-blue-400 to-indigo-600",
    "from-purple-400 to-pink-600",
    "from-orange-400 to-red-600",
    "from-yellow-400 to-orange-500",
    "from-cyan-400 to-blue-500",
];

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
        if (el) el.classList.remove("bg-slate-100", "text-slate-900", "font-semibold");
    });
    // Mostra a aba selecionada
    const tabId = tabs[navId];
    const tab = document.getElementById(tabId);
    if (tab) tab.classList.remove("hidden");
    // Ativa o link
    const nav = document.getElementById(navId);
    if (nav) nav.classList.add("bg-slate-100", "text-slate-900", "font-semibold");
}

// Liga os cliques da sidebar
Object.keys(tabs).forEach(navId => {
    document.getElementById(navId)?.addEventListener("click", (e) => {
        e.preventDefault();
        showTab(navId);
        // Carrega dados da aba ao trocar
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
        const banner = document.getElementById("status-banner");
        const bannerText = document.getElementById("status-text");
        if (data.sync_pending_count > 0) {
            banner?.classList.replace("bg-emerald-50", "bg-yellow-50");
            if (bannerText) bannerText.textContent = `${data.sync_pending_count} item(s) aguardando sincronização...`;
        } else {
            banner?.classList.replace("bg-yellow-50", "bg-emerald-50");
            if (bannerText) bannerText.textContent = "Tudo certo. Seus saves estão protegidos e sincronizados.";
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
        container.innerHTML = `<p class="text-sm text-slate-400 px-1">Nenhum jogo detectado ainda.</p>`;
        return;
    }
    container.innerHTML = locations.map((loc, i) => {
        const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
        const grad = gradients[i % gradients.length];
        return `
        <div class="flex items-center gap-3 py-2.5 px-1 border-b border-slate-100 last:border-0">
            <div class="w-9 h-9 rounded-xl bg-gradient-to-br ${grad} flex items-center justify-center text-white text-xs font-bold flex-shrink-0">
                ${initials}
            </div>
            <div class="flex-1 min-w-0">
                <p class="text-sm font-semibold text-slate-800 truncate">${loc.game_name || "Jogo"}</p>
                <p class="text-xs text-slate-400">${loc.files_count ?? 0} arquivo(s) · ${timeAgo(loc.last_seen_at)}</p>
            </div>
            <span class="text-xs font-medium px-2 py-0.5 rounded-full bg-emerald-100 text-emerald-700 flex-shrink-0">Sincronizado</span>
        </div>`;
    }).join("");
}

// ── Aba: Jogos ────────────────────────────────────────────────────────────────

async function loadJogos() {
    const container = document.getElementById("jogos-list");
    if (!container) return;
    container.innerHTML = `<p class="text-sm text-slate-400">Carregando...</p>`;
    try {
        const locs = await invoke("get_locations");
        if (!locs || locs.length === 0) {
            container.innerHTML = `<p class="text-sm text-slate-400">Nenhum jogo detectado. Execute um scan.</p>`;
            return;
        }
        container.innerHTML = locs.map((loc, i) => {
            const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
            const grad = gradients[i % gradients.length];
            return `
            <div class="bg-white border border-slate-200/90 rounded-2xl p-4 flex items-center gap-4 shadow-xs">
                <div class="w-12 h-12 rounded-xl bg-gradient-to-br ${grad} flex items-center justify-center text-white text-sm font-bold flex-shrink-0">
                    ${initials}
                </div>
                <div class="flex-1 min-w-0">
                    <p class="font-semibold text-slate-800">${loc.game_name || "Jogo Desconhecido"}</p>
                    <p class="text-xs text-slate-500 truncate mt-0.5">${loc.path || ""}</p>
                    <p class="text-xs text-slate-400 mt-1">${loc.files_count ?? 0} arquivo(s) monitorado(s)</p>
                </div>
                <span class="text-xs font-medium px-2.5 py-1 rounded-full bg-emerald-100 text-emerald-700 flex-shrink-0">Ativo</span>
            </div>`;
        }).join("");
    } catch (e) {
        container.innerHTML = `<p class="text-sm text-red-500">Erro: ${e}</p>`;
    }
}

// ── Aba: Atividade ────────────────────────────────────────────────────────────

async function loadAtividade() {
    const container = document.getElementById("atividade-list");
    if (!container) return;
    container.innerHTML = `<p class="text-sm text-slate-400">Carregando histórico...</p>`;
    try {
        const snapshots = await invoke("get_snapshots");
        if (!snapshots || snapshots.length === 0) {
            container.innerHTML = `<p class="text-sm text-slate-400">Nenhum snapshot encontrado ainda.</p>`;
            return;
        }
        container.innerHTML = snapshots.map(s => `
            <div class="bg-white border border-slate-200/90 rounded-xl p-4 shadow-xs flex items-start gap-3">
                <div class="w-2 h-2 rounded-full bg-emerald-400 mt-2 flex-shrink-0"></div>
                <div class="flex-1 min-w-0">
                    <p class="font-medium text-slate-800 text-sm">${s.game_name || "Arquivo de Save"}</p>
                    <p class="text-xs text-slate-500 truncate mt-0.5 font-mono">${(s.blake3_hash || "").substring(0, 16)}…</p>
                    <p class="text-xs text-slate-400 mt-1">${s.file_size_bytes ? formatBytes(s.file_size_bytes) : ""} · ${timeAgo(s.captured_at)}</p>
                </div>
                <button
                    onclick="restoreSnapshot('${s.snapshot_id}', '${(s.game_name || "").replace(/'/g, "")}')"
                    class="text-xs px-3 py-1.5 rounded-lg border border-slate-200 text-slate-600 hover:bg-slate-100 transition-colors flex-shrink-0">
                    Restaurar
                </button>
            </div>`).join("");
    } catch (e) {
        container.innerHTML = `<p class="text-sm text-red-500">Erro: ${e}</p>`;
    }
}

function formatBytes(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1048576).toFixed(1)} MB`;
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
            <div class="bg-white border border-slate-200/90 rounded-2xl p-5 shadow-xs space-y-3">
                <div class="flex items-center gap-3">
                    <div class="w-10 h-10 rounded-xl bg-blue-50 flex items-center justify-center text-blue-500 text-xl">☁️</div>
                    <div>
                        <p class="font-semibold text-slate-800">Google Drive</p>
                        <p class="text-sm text-slate-500">${data.app_status || "Verificando..."}</p>
                    </div>
                </div>
                <div class="text-sm text-slate-600 space-y-1 pt-1 border-t border-slate-100">
                    <p>📦 Itens na fila de sync: <strong>${data.sync_pending_count ?? 0}</strong></p>
                    <p>🔄 Último sync: <strong>${formatHour(data.last_reconciliation_time)}</strong></p>
                </div>
            </div>`;
    } catch (e) {
        container.innerHTML = `<p class="text-sm text-red-500">Erro ao carregar status da nuvem: ${e}</p>`;
    }
}

// ── Aba: Configurações ────────────────────────────────────────────────────────

async function loadConfig() {
    const container = document.getElementById("config-info");
    if (!container) return;
    try {
        const data = await invoke("get_dashboard_status");
        container.innerHTML = `
            <div class="space-y-3">
                <div class="bg-white border border-slate-200/90 rounded-xl p-4 shadow-xs">
                    <p class="text-xs font-semibold text-slate-500 uppercase tracking-wider mb-1">Banco de Dados</p>
                    <p class="text-sm text-slate-700 font-mono break-all">${data.db_path || "—"}</p>
                </div>
                <div class="bg-white border border-slate-200/90 rounded-xl p-4 shadow-xs">
                    <p class="text-xs font-semibold text-slate-500 uppercase tracking-wider mb-1">Pasta de Backups</p>
                    <p class="text-sm text-slate-700 font-mono break-all">${data.backup_root || "—"}</p>
                </div>
                <div class="bg-white border border-slate-200/90 rounded-xl p-4 shadow-xs">
                    <p class="text-xs font-semibold text-slate-500 uppercase tracking-wider mb-1">Intervalo de Reconciliação</p>
                    <p class="text-sm text-slate-700">${data.reconciliation_interval_secs ?? 30}s</p>
                </div>
                <div class="bg-white border border-slate-200/90 rounded-xl p-4 shadow-xs">
                    <p class="text-xs font-semibold text-slate-500 uppercase tracking-wider mb-1">Status do Watcher</p>
                    <p class="text-sm text-slate-700">${data.watcher_status || "—"}</p>
                </div>
            </div>`;
    } catch (e) {
        container.innerHTML = `<p class="text-sm text-red-500">Erro: ${e}</p>`;
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
