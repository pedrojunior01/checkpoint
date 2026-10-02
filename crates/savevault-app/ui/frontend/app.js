const hasTauri = typeof window !== "undefined" && !!(window.__TAURI__ && window.__TAURI__.core);
const appWindow = (typeof window !== "undefined" && window.__TAURI__ && window.__TAURI__.window)
    ? window.__TAURI__.window.getCurrentWindow()
    : {
        minimize: () => {},
        toggleMaximize: () => {},
        close: () => window.close(),
    };

async function invoke(command, args = {}) {
    if (hasTauri) {
        return window.__TAURI__.core.invoke(command, args);
    }
    switch (command) {
        case "get_dashboard_status": {
            const res = await fetch("/api/dashboard");
            return await res.json();
        }
        case "get_locations": {
            const res = await fetch("/api/locations");
            return await res.json();
        }
        case "get_snapshots": {
            const res = await fetch("/api/snapshots");
            return await res.json();
        }
        case "get_safety_backups": {
            const res = await fetch("/api/safety-backups");
            return await res.json();
        }
        case "get_reconciliation_status": {
            const res = await fetch("/api/reconciliation");
            return await res.json();
        }
        case "trigger_reconcile": {
            const res = await fetch("/api/reconcile-now", {
                method: "POST",
                headers: { "X-SaveVault-Request": "1" },
            });
            const data = await res.json();
            return `${data.files_changed || 0} arquivo(s) modificado(s)`;
        }
        case "restore_snapshot": {
            const res = await fetch("/api/restore", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    snapshot_id: args.snapshotId || args.snapshot_id,
                }),
            });
            return await res.json();
        }
        case "get_setup_status": {
            const res = await fetch("/api/setup/status");
            return await res.json();
        }
        case "run_setup_discovery": {
            const res = await fetch("/api/setup/scan", {
                method: "POST",
                headers: { "X-SaveVault-Request": "1" },
            });
            const data = await res.json();
            return data.games || [];
        }
        case "get_cloud_folders": {
            const res = await fetch("/api/setup/cloud-folders");
            return await res.json();
        }
        case "save_setup_cloud": {
            const res = await fetch("/api/setup/login-cloud", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    provider: args.provider,
                    local_sync_path: args.localSyncPath || args.local_sync_path,
                }),
            });
            return await res.json();
        }
        case "complete_setup": {
            const res = await fetch("/api/setup/complete", {
                method: "POST",
                headers: { "Content-Type": "application/json", "X-SaveVault-Request": "1" },
                body: JSON.stringify({
                    provider: args.provider,
                    autostart: args.autostart,
                    notifications: args.notifications,
                    local_sync_path: args.localSyncPath || args.local_sync_path,
                }),
            });
            return await res.json();
        }
        default:
            throw new Error(`Comando não implementado: ${command}`);
    }
}

// ── Utilitários ───────────────────────────────────────────────────────────────

function escapeHtml(str) {
    if (str === null || str === undefined) return "";
    return String(str)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}

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

let allJogosLocations = [];

async function loadJogos() {
    const container = document.getElementById("jogos-list");
    if (!container) return;
    container.innerHTML = `<p class="text-sm text-slate-400">Carregando...</p>`;
    try {
        const locs = await invoke("get_locations");
        allJogosLocations = locs || [];
        const searchInput = document.getElementById("jogos-search-input");
        const currentQuery = searchInput ? searchInput.value.trim() : "";
        renderJogosList(currentQuery);
    } catch (e) {
        container.innerHTML = `<p class="text-sm text-red-500">Erro: ${e}</p>`;
    }
}

function renderJogosList(query = "") {
    const container = document.getElementById("jogos-list");
    const statsEl = document.getElementById("jogos-search-stats");
    const clearBtn = document.getElementById("jogos-search-clear");
    if (!container) return;

    const trimmedQuery = (query || "").trim().toLowerCase();

    if (clearBtn) {
        if (trimmedQuery.length > 0) {
            clearBtn.classList.remove("hidden");
        } else {
            clearBtn.classList.add("hidden");
        }
    }

    if (!allJogosLocations || allJogosLocations.length === 0) {
        if (statsEl) statsEl.classList.add("hidden");
        container.innerHTML = `<p class="text-sm text-slate-400">Nenhum jogo detectado. Execute um scan.</p>`;
        return;
    }

    let filtered = allJogosLocations;
    if (trimmedQuery.length > 0) {
        filtered = allJogosLocations.filter(loc => {
            const name = (loc.game_name || "").toLowerCase();
            const path = (loc.path || "").toLowerCase();
            return name.includes(trimmedQuery) || path.includes(trimmedQuery);
        });

        if (statsEl) {
            statsEl.classList.remove("hidden");
            statsEl.textContent = `Mostrando ${filtered.length} de ${allJogosLocations.length} jogo(s) encontrado(s) para "${query}"`;
        }
    } else {
        if (statsEl) statsEl.classList.add("hidden");
    }

    if (filtered.length === 0) {
        container.innerHTML = `
            <div class="bg-white border border-slate-200/90 rounded-2xl p-8 text-center shadow-xs">
                <div class="text-3xl mb-2">🔍</div>
                <p class="font-semibold text-slate-800 text-sm">Nenhum jogo encontrado</p>
                <p class="text-xs text-slate-500 mt-1">Nenhum resultado corresponde à busca "<strong>${escapeHtml(query)}</strong>".</p>
                <button onclick="clearJogosSearch()" class="mt-3 px-3 py-1.5 bg-slate-100 hover:bg-slate-200 text-slate-700 text-xs font-semibold rounded-lg transition-colors">
                    Limpar pesquisa
                </button>
            </div>
        `;
        return;
    }

    container.innerHTML = filtered.map((loc, i) => {
        const initials = (loc.game_name || "?").substring(0, 2).toUpperCase();
        const grad = gradients[i % gradients.length];
        return `
        <div class="bg-white border border-slate-200/90 rounded-2xl p-4 flex items-center gap-4 shadow-xs hover:border-slate-300 transition-colors">
            <div class="w-12 h-12 rounded-xl bg-gradient-to-br ${grad} flex items-center justify-center text-white text-sm font-bold flex-shrink-0">
                ${initials}
            </div>
            <div class="flex-1 min-w-0">
                <p class="font-semibold text-slate-800">${escapeHtml(loc.game_name || "Jogo Desconhecido")}</p>
                <p class="text-xs text-slate-500 truncate mt-0.5 font-mono">${escapeHtml(loc.path || "")}</p>
                <p class="text-xs text-slate-400 mt-1">${loc.files_count ?? 0} arquivo(s) monitorado(s)</p>
            </div>
            <span class="text-xs font-medium px-2.5 py-1 rounded-full bg-emerald-100 text-emerald-700 flex-shrink-0">Ativo</span>
        </div>`;
    }).join("");
}

function clearJogosSearch() {
    const input = document.getElementById("jogos-search-input");
    if (input) {
        input.value = "";
        input.focus();
    }
    renderJogosList("");
}

// ── Aba: Atividade ────────────────────────────────────────────────────────────

let allSnapshots = [];

async function loadAtividade() {
    const container = document.getElementById("atividade-list");
    if (!container) return;
    container.innerHTML = `<p class="text-sm text-slate-400">Carregando histórico...</p>`;
    try {
        const snapshots = await invoke("get_snapshots");
        allSnapshots = snapshots || [];
        const searchInput = document.getElementById("atividade-search-input");
        const currentQuery = searchInput ? searchInput.value.trim() : "";
        renderAtividadeList(currentQuery);
    } catch (e) {
        container.innerHTML = `<p class="text-sm text-red-500">Erro: ${escapeHtml(e)}</p>`;
    }
}

function renderAtividadeList(query = "") {
    const container = document.getElementById("atividade-list");
    const statsEl = document.getElementById("atividade-search-stats");
    const clearBtn = document.getElementById("atividade-search-clear");
    if (!container) return;

    const trimmedQuery = (query || "").trim().toLowerCase();

    if (clearBtn) {
        if (trimmedQuery.length > 0) {
            clearBtn.classList.remove("hidden");
        } else {
            clearBtn.classList.add("hidden");
        }
    }

    if (!allSnapshots || allSnapshots.length === 0) {
        if (statsEl) statsEl.classList.add("hidden");
        container.innerHTML = `
            <div class="bg-white border border-slate-200/90 rounded-2xl p-8 text-center shadow-xs">
                <div class="text-3xl mb-2">📦</div>
                <p class="font-semibold text-slate-800 text-sm">Nenhum snapshot capturado ainda</p>
                <p class="text-xs text-slate-500 mt-1">Quando você jogar ou salvar o progresso, o SaveVault criará pontos de restauração aqui.</p>
            </div>
        `;
        return;
    }

    let filtered = allSnapshots;
    if (trimmedQuery.length > 0) {
        filtered = allSnapshots.filter(s => {
            const game = (s.game_name || "").toLowerCase();
            const file = (s.file_name || "").toLowerCase();
            const path = (s.file_path || "").toLowerCase();
            const version = `v${s.version || ""}`.toLowerCase();
            const hash = (s.hash || "").toLowerCase();
            return game.includes(trimmedQuery)
                || file.includes(trimmedQuery)
                || path.includes(trimmedQuery)
                || version.includes(trimmedQuery)
                || hash.includes(trimmedQuery);
        });

        if (statsEl) {
            statsEl.classList.remove("hidden");
            statsEl.textContent = `Mostrando ${filtered.length} de ${allSnapshots.length} snapshot(s) encontrado(s) para "${query}"`;
        }
    } else {
        if (statsEl) statsEl.classList.add("hidden");
    }

    if (filtered.length === 0) {
        container.innerHTML = `
            <div class="bg-white border border-slate-200/90 rounded-2xl p-8 text-center shadow-xs">
                <div class="text-3xl mb-2">🔍</div>
                <p class="font-semibold text-slate-800 text-sm">Nenhum snapshot encontrado</p>
                <p class="text-xs text-slate-500 mt-1">Nenhum resultado corresponde à busca "<strong>${escapeHtml(query)}</strong>".</p>
                <button onclick="clearAtividadeSearch()" class="mt-3 px-3 py-1.5 bg-slate-100 hover:bg-slate-200 text-slate-700 text-xs font-semibold rounded-lg transition-colors">
                    Limpar pesquisa
                </button>
            </div>
        `;
        return;
    }

    container.innerHTML = filtered.map(s => {
        const gameTitle = s.game_name || "Jogo Desconhecido";
        const fileName = s.file_name || "save";
        const versionLabel = `v${s.version || 1}`;
        const sizeLabel = s.size_formatted || (s.size_bytes ? formatBytes(s.size_bytes) : "—");
        const hashShort = s.short_hash || (s.hash ? s.hash.substring(0, 10) : "—");
        const formattedDate = formatHour(s.created_at);
        const relativeDate = timeAgo(s.created_at);

        return `
            <div class="bg-white border border-slate-200/90 rounded-2xl p-4 shadow-xs hover:border-slate-300 transition-colors flex flex-col sm:flex-row sm:items-center justify-between gap-3">
                <div class="flex items-start gap-3 min-w-0">
                    <div class="w-9 h-9 rounded-xl bg-emerald-50 border border-emerald-200 flex items-center justify-center text-emerald-600 font-bold text-xs shrink-0 mt-0.5">
                        ${versionLabel}
                    </div>
                    <div class="min-w-0 flex-1">
                        <div class="flex items-center gap-2 flex-wrap">
                            <span class="font-bold text-slate-900 text-sm">${escapeHtml(gameTitle)}</span>
                            <span class="px-2 py-0.5 rounded-full text-xs font-semibold bg-emerald-100 text-emerald-800">
                                ${versionLabel}
                            </span>
                            <span class="text-xs font-mono font-medium text-slate-600 bg-slate-100 px-2 py-0.5 rounded-md truncate max-w-xs" title="${escapeHtml(s.file_path || fileName)}">
                                📄 ${escapeHtml(fileName)}
                            </span>
                        </div>
                        <div class="text-xs text-slate-500 mt-1 flex items-center gap-2 flex-wrap">
                            <span>📦 ${escapeHtml(sizeLabel)}</span>
                            <span>•</span>
                            <span title="${escapeHtml(s.created_at || "")}">🕒 ${escapeHtml(relativeDate)} (${escapeHtml(formattedDate)})</span>
                            <span>•</span>
                            <span class="font-mono text-slate-400" title="Hash Blake3: ${escapeHtml(s.hash || "")}">#${escapeHtml(hashShort)}</span>
                        </div>
                    </div>
                </div>
                <div class="flex items-center gap-2 shrink-0 self-end sm:self-center">
                    <button
                        onclick="restoreSnapshot('${s.id}', '${escapeHtml(gameTitle).replace(/'/g, "\\'")}', '${versionLabel}', '${escapeHtml(fileName).replace(/'/g, "\\'")}')"
                        class="px-3.5 py-1.5 rounded-xl bg-slate-800 hover:bg-slate-700 text-white text-xs font-semibold shadow-xs transition-colors flex items-center gap-1.5 cursor-pointer">
                        <span>↺</span> Restaurar Save
                    </button>
                </div>
            </div>`;
    }).join("");
}

function clearAtividadeSearch() {
    const input = document.getElementById("atividade-search-input");
    if (input) {
        input.value = "";
        input.focus();
    }
    renderAtividadeList("");
}

function formatBytes(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1048576).toFixed(1)} MB`;
}

async function restoreSnapshot(id, gameName, version, fileName) {
    const promptMsg = `Deseja restaurar o snapshot ${version} de "${gameName}"?\nArquivo: ${fileName}\n\nUm backup de segurança do seu save atual será criado automaticamente antes da restauração.`;
    if (!confirm(promptMsg)) return;

    try {
        const result = await invoke("restore_snapshot", { snapshotId: id, snapshot_id: id });
        alert(`Restauração concluída com sucesso!\n\nArquivo restaurado: ${result.restored_path || result.dest_path || "OK"}\nSafety Backup: ${result.safety_backup_path || "Criado com sucesso"}`);
        loadAtividade();
    } catch (e) {
        alert(`Erro ao restaurar snapshot: ${e}`);
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
                        <p class="font-semibold text-slate-800">Pasta Local da Nuvem</p>
                        <p class="text-sm text-slate-500">${data.app_status || "Ativo"}</p>
                    </div>
                </div>
                <div class="text-sm text-slate-600 space-y-1.5 pt-1 border-t border-slate-100">
                    <p>📁 Provedor: <strong>Sincronização via Pasta Local (OneDrive / Drive / Dropbox)</strong></p>
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

// ── Assistente de Primeiro Acesso (Setup Wizard) ──────────────────────────────

let currentWizardStep = 1;
let wizardDiscoveredGames = [];

async function checkInitialSetup() {
    try {
        const status = await invoke("get_setup_status");
        if (status && !status.setup_completed) {
            openSetupWizard();
        }
    } catch (e) {
        console.warn("Não foi possível verificar status do setup inicial:", e);
    }
}

async function openSetupWizard() {
    currentWizardStep = 1;
    const modal = document.getElementById("setup-wizard-modal");
    if (modal) {
        modal.classList.remove("hidden");
    }
    wizardGoToStep(1);
    await loadWizardSetupStatus();
}

function closeSetupWizard() {
    const modal = document.getElementById("setup-wizard-modal");
    if (modal) {
        modal.classList.add("hidden");
    }
}

async function loadWizardSetupStatus() {
    try {
        const data = await invoke("get_setup_status");
        const foldersContainer = document.getElementById("wizard-detected-folders");
        const inputCloudPath = document.getElementById("wizard-input-cloud-path");

        if (foldersContainer && data.detected_cloud_folders) {
            if (data.detected_cloud_folders.length > 0) {
                let html = "";
                data.detected_cloud_folders.forEach(df => {
                    const safeFolder = (df.savevault_folder || "").replace(/\\/g, "\\\\").replace(/'/g, "\\'");
                    const icon = (df.name || "").toLowerCase().includes("drive") ? "📁" : "☁️";
                    html += `
                        <div class="flex items-center justify-between p-2.5 bg-slate-50 border border-slate-200/90 rounded-xl text-xs">
                            <div class="flex items-center gap-2 overflow-hidden mr-2">
                                <span class="text-base">${icon}</span>
                                <div class="truncate">
                                    <p class="font-bold text-slate-800">${escapeHtml(df.name)}</p>
                                    <p class="text-[11px] text-slate-500 font-mono truncate">${escapeHtml(df.savevault_folder)}</p>
                                </div>
                            </div>
                            <button onclick="setWizardCloudPath('${safeFolder}')" class="px-2.5 py-1 bg-white hover:bg-slate-100 text-slate-700 font-semibold border border-slate-300/80 rounded-md transition-colors shrink-0">
                                Usar Esta
                            </button>
                        </div>
                    `;
                });
                foldersContainer.innerHTML = html;
                if (inputCloudPath && !inputCloudPath.value) {
                    inputCloudPath.value = data.local_sync_path || data.detected_cloud_folders[0].savevault_folder;
                }
            } else {
                foldersContainer.innerHTML = `
                    <div class="p-3 bg-amber-50 border border-amber-200 rounded-xl text-amber-800 text-[11px]">
                        Nenhuma pasta de nuvem padrão (Google Drive Desktop, OneDrive ou Dropbox) foi detectada automaticamente. Você pode digitar o caminho da sua pasta no campo abaixo.
                    </div>
                `;
            }
        }

        if (inputCloudPath && data.local_sync_path && !inputCloudPath.value) {
            inputCloudPath.value = data.local_sync_path;
        }

        const optAutostart = document.getElementById("wizard-opt-autostart");
        if (optAutostart && typeof data.autostart_enabled === "boolean") {
            optAutostart.checked = data.autostart_enabled;
        }

        const optNotifications = document.getElementById("wizard-opt-notifications");
        if (optNotifications && typeof data.notifications_enabled === "boolean") {
            optNotifications.checked = data.notifications_enabled;
        }
    } catch (e) {
        console.error("Erro ao carregar status do wizard:", e);
    }
}

function setWizardCloudPath(path) {
    const input = document.getElementById("wizard-input-cloud-path");
    if (input) {
        input.value = path;
    }
}

async function runWizardDiscovery() {
    const btn = document.getElementById("btn-wizard-scan");
    const container = document.getElementById("wizard-games-container");

    if (btn) {
        btn.disabled = true;
        btn.innerHTML = `<span class="animate-spin inline-block mr-1">🔄</span> Escaneando...`;
    }

    try {
        const games = await invoke("run_setup_discovery");
        wizardDiscoveredGames = games || [];

        if (container) {
            if (wizardDiscoveredGames.length === 0) {
                container.innerHTML = `
                    <div class="text-center py-6 text-slate-400 text-xs">
                        Nenhum save de jogo foi encontrado nas pastas padrão (Steam/GOG). Você poderá adicionar locais manualmente na aba "Saves" depois.
                    </div>
                `;
            } else {
                let html = "";
                wizardDiscoveredGames.forEach(g => {
                    html += `
                        <div class="p-2.5 bg-white border border-slate-200/90 rounded-lg text-xs flex items-center justify-between">
                            <div class="overflow-hidden mr-2">
                                <span class="font-bold text-slate-800">🎮 ${escapeHtml(g.name)}</span>
                                <span class="ml-1 text-[10px] uppercase font-bold text-slate-400 bg-slate-100 px-1.5 py-0.5 rounded">${escapeHtml(g.platform)}</span>
                                <p class="text-[11px] text-slate-500 font-mono truncate mt-0.5">${escapeHtml(g.save_path)}</p>
                            </div>
                            <span class="text-emerald-600 font-semibold text-[11px] shrink-0">Pronto</span>
                        </div>
                    `;
                });
                container.innerHTML = html;
            }
        }
    } catch (e) {
        if (container) {
            container.innerHTML = `<p class="text-xs text-red-500">Erro na detecção: ${e}</p>`;
        }
    } finally {
        if (btn) {
            btn.disabled = false;
            btn.innerHTML = `<span>🔍</span> Re-escanear`;
        }
    }
}

function wizardGoToStep(step) {
    currentWizardStep = step;

    const subtitles = [
        "Passo 1 de 5: Boas-vindas",
        "Passo 2 de 5: Pasta de Sincronização na Nuvem",
        "Passo 3 de 5: Varredura de Jogos",
        "Passo 4 de 5: Preferências do Sistema",
        "Passo 5 de 5: Conclusão e Ativação"
    ];

    const subEl = document.getElementById("wizard-step-subtitle");
    if (subEl) {
        subEl.textContent = subtitles[step - 1] || "";
    }

    for (let i = 1; i <= 5; i++) {
        const pill = document.getElementById(`pill-step-${i}`);
        if (pill) {
            if (i === step) {
                pill.className = "wizard-step-pill px-2.5 py-1 rounded-full bg-slate-800 text-white font-semibold";
            } else if (i < step) {
                pill.className = "wizard-step-pill px-2.5 py-1 rounded-full bg-emerald-100 text-emerald-800 font-semibold";
            } else {
                pill.className = "wizard-step-pill px-2.5 py-1 rounded-full bg-slate-100 text-slate-500";
            }
        }

        const pane = document.getElementById(`wizard-pane-${i}`);
        if (pane) {
            if (i === step) {
                pane.classList.remove("hidden");
            } else {
                pane.classList.add("hidden");
            }
        }
    }

    const prevBtn = document.getElementById("btn-wizard-prev");
    const nextBtn = document.getElementById("btn-wizard-next");

    if (prevBtn) {
        if (step === 1) {
            prevBtn.classList.add("hidden");
        } else {
            prevBtn.classList.remove("hidden");
        }
    }

    if (nextBtn) {
        if (step === 5) {
            nextBtn.innerHTML = `<span>Concluir e Proteger</span> 🚀`;
            nextBtn.className = "px-4 py-2 bg-emerald-600 hover:bg-emerald-500 text-white text-xs font-semibold rounded-lg shadow-xs transition-colors flex items-center gap-1.5";
        } else {
            nextBtn.innerHTML = `<span>Avançar</span> →`;
            nextBtn.className = "px-4 py-2 bg-slate-800 hover:bg-slate-700 text-white text-xs font-semibold rounded-lg shadow-xs transition-colors flex items-center gap-1.5";
        }
    }

    if (step === 5) {
        const cloudPath = document.getElementById("wizard-input-cloud-path")?.value?.trim() || "Pasta padrão local";
        const sumCloud = document.getElementById("wizard-summary-cloud");
        if (sumCloud) sumCloud.textContent = cloudPath;

        const sumGames = document.getElementById("wizard-summary-games");
        if (sumGames) sumGames.textContent = `${wizardDiscoveredGames.length} jogo(s) detectado(s)`;

        const autostart = document.getElementById("wizard-opt-autostart")?.checked;
        const sumAutostart = document.getElementById("wizard-summary-autostart");
        if (sumAutostart) sumAutostart.textContent = autostart ? "Ativado" : "Desativado";

        const notif = document.getElementById("wizard-opt-notifications")?.checked;
        const sumNotif = document.getElementById("wizard-summary-notifications");
        if (sumNotif) sumNotif.textContent = notif ? "Ativadas" : "Desativadas";
    }
}

async function wizardNextStep() {
    if (currentWizardStep < 4) {
        wizardGoToStep(currentWizardStep + 1);
    } else if (currentWizardStep === 4) {
        const autostart = document.getElementById("wizard-opt-autostart")?.checked ?? false;
        const notifications = document.getElementById("wizard-opt-notifications")?.checked ?? true;
        const localSyncPath = document.getElementById("wizard-input-cloud-path")?.value?.trim() || null;

        try {
            await invoke("complete_setup", {
                provider: "local_folder",
                autostart,
                notifications,
                localSyncPath,
            });
        } catch (e) {
            console.error("Erro ao registrar conclusão do setup:", e);
        }
        wizardGoToStep(5);
    } else if (currentWizardStep === 5) {
        closeSetupWizard();
        loadInicio();
    }
}

function wizardPrevStep() {
    if (currentWizardStep > 1) {
        wizardGoToStep(currentWizardStep - 1);
    }
}

// ── Init ──────────────────────────────────────────────────────────────────────

document.addEventListener("DOMContentLoaded", () => {
    showTab("nav-inicio");
    loadInicio();
    checkInitialSetup();

    // Listener da barra de pesquisa de jogos
    const searchInput = document.getElementById("jogos-search-input");
    searchInput?.addEventListener("input", (e) => {
        renderJogosList(e.target.value);
    });
    searchInput?.addEventListener("keydown", (e) => {
        if (e.key === "Escape") {
            clearJogosSearch();
        }
    });
    document.getElementById("jogos-search-clear")?.addEventListener("click", () => {
        clearJogosSearch();
    });

    // Atualiza dashboard a cada 10s
    setInterval(loadInicio, 10000);
});
