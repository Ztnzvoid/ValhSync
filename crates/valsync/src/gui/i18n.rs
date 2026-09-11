//! Two languages, one enum. French first because that is who plays on the
//! server this was built for; English because the project is public.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Fr,
    En,
}

impl Lang {
    /// From the saved setting, else the system locale, else English.
    pub fn detect(setting: Option<&str>) -> Self {
        let code = setting
            .map(str::to_string)
            .or_else(sys_locale::get_locale)
            .unwrap_or_default()
            .to_lowercase();
        if code.starts_with("fr") {
            Self::Fr
        } else {
            Self::En
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Fr => "fr",
            Self::En => "en",
        }
    }

    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Fr => Self::En,
            Self::En => Self::Fr,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Subtitle,
    AddServer,
    NoServerHint,
    Checking,
    UpToDate,
    FirstSync,
    Pending,
    Play,
    PlayVanilla,
    ModsDisabled,
    ModsEnabled,
    Rollback,
    OpenQuarantine,
    GameFolder,
    GameFolderHint,
    Apply,
    Cancel,
    Close,
    InvitePrompt,
    InviteHint,
    Added,
    Launching,
    SyncDone,
    RolledBack,
    Downloading,
    Applying,
    Contacting,
    PlanInstall,
    PlanUpdate,
    PlanRemove,
    PlanQuarantine,
    PlanDownload,
    ConfirmTitle,
    ConfirmBody,
    Forget,
    Refresh,
    Settings,
    Language,
    Busy,
    KeyFingerprint,
    TrustLine,
    Installed,
    Nothing,
}

pub fn text(lang: Lang, key: Key) -> &'static str {
    match lang {
        Lang::Fr => fr(key),
        Lang::En => en(key),
    }
}

fn fr(key: Key) -> &'static str {
    match key {
        Key::Subtitle => "Vos mods, alignés sur le serveur. Puis on joue.",
        Key::AddServer => "Ajouter un serveur",
        Key::NoServerHint => {
            "Collez le code d'invitation que l'admin vous a envoyé, ou placez son fichier valsync-invite.txt à côté de valsync.exe."
        }
        Key::Checking => "Vérification auprès du serveur…",
        Key::UpToDate => "À jour",
        Key::FirstSync => "Première synchronisation",
        Key::Pending => "Mise à jour disponible",
        Key::Play => "JOUER",
        Key::PlayVanilla => "Jouer sans mods",
        Key::ModsDisabled => "Mods désactivés. La prochaine synchronisation les rétablira.",
        Key::ModsEnabled => "Mods réactivés.",
        Key::Rollback => "Revenir à la version précédente",
        Key::OpenQuarantine => "Voir la quarantaine",
        Key::GameFolder => "Dossier du jeu",
        Key::GameFolderHint => {
            "Dossier contenant valheim.exe. Vide = détection automatique via Steam."
        }
        Key::Apply => "Appliquer",
        Key::Cancel => "Annuler",
        Key::Close => "Fermer",
        Key::InvitePrompt => "Code d'invitation",
        Key::InviteHint => "valsync1:…",
        Key::Added => "Serveur ajouté",
        Key::Launching => "Valheim démarre via Steam…",
        Key::SyncDone => "Synchronisation terminée",
        Key::RolledBack => "Version précédente restaurée",
        Key::Downloading => "Téléchargement",
        Key::Applying => "Application des changements…",
        Key::Contacting => "Connexion au serveur…",
        Key::PlanInstall => "à installer",
        Key::PlanUpdate => "à mettre à jour",
        Key::PlanRemove => "à retirer",
        Key::PlanQuarantine => "à mettre en quarantaine",
        Key::PlanDownload => "à télécharger",
        Key::ConfirmTitle => "Ce que ValSync va faire",
        Key::ConfirmBody => {
            "Première synchronisation avec ce serveur. Les fichiers inconnus trouvés dans les dossiers de mods seront déplacés en quarantaine, pas supprimés. Une sauvegarde est faite avant tout changement."
        }
        Key::Forget => "Oublier ce serveur",
        Key::Refresh => "Revérifier",
        Key::Settings => "Réglages",
        Key::Language => "Langue",
        Key::Busy => "Opération en cours…",
        Key::KeyFingerprint => "Empreinte de la clé",
        Key::TrustLine => {
            "Fichiers signés par ce serveur et vérifiés un par un. ValSync n'écrit que dans le dossier du jeu (BepInEx) et n'exécute rien lui-même : c'est Steam qui lance Valheim."
        }
        Key::Installed => "installés",
        Key::Nothing => "Rien à faire.",
    }
}

fn en(key: Key) -> &'static str {
    match key {
        Key::Subtitle => "Your mods, matched to the server. Then play.",
        Key::AddServer => "Add a server",
        Key::NoServerHint => {
            "Paste the invite code your admin sent you, or drop their valsync-invite.txt next to valsync.exe."
        }
        Key::Checking => "Checking with the server…",
        Key::UpToDate => "Up to date",
        Key::FirstSync => "First sync",
        Key::Pending => "Update available",
        Key::Play => "PLAY",
        Key::PlayVanilla => "Play without mods",
        Key::ModsDisabled => "Mods disabled. The next sync will restore them.",
        Key::ModsEnabled => "Mods enabled again.",
        Key::Rollback => "Go back to the previous version",
        Key::OpenQuarantine => "Open quarantine",
        Key::GameFolder => "Game folder",
        Key::GameFolderHint => {
            "Folder containing valheim.exe. Empty = automatic detection via Steam."
        }
        Key::Apply => "Apply",
        Key::Cancel => "Cancel",
        Key::Close => "Close",
        Key::InvitePrompt => "Invite code",
        Key::InviteHint => "valsync1:…",
        Key::Added => "Server added",
        Key::Launching => "Valheim is starting via Steam…",
        Key::SyncDone => "Sync complete",
        Key::RolledBack => "Previous version restored",
        Key::Downloading => "Downloading",
        Key::Applying => "Applying changes…",
        Key::Contacting => "Contacting the server…",
        Key::PlanInstall => "to install",
        Key::PlanUpdate => "to update",
        Key::PlanRemove => "to remove",
        Key::PlanQuarantine => "to quarantine",
        Key::PlanDownload => "to download",
        Key::ConfirmTitle => "What ValSync is about to do",
        Key::ConfirmBody => {
            "First sync with this server. Unknown files found in the mod folders are moved to a quarantine, never deleted. A backup is taken before anything changes."
        }
        Key::Forget => "Forget this server",
        Key::Refresh => "Check again",
        Key::Settings => "Settings",
        Key::Language => "Language",
        Key::Busy => "Working…",
        Key::KeyFingerprint => "Key fingerprint",
        Key::TrustLine => {
            "Files are signed by this server and verified one by one. ValSync only writes inside the game folder (BepInEx) and runs nothing itself: Steam starts Valheim."
        }
        Key::Installed => "installed",
        Key::Nothing => "Nothing to do.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_prefers_setting() {
        assert_eq!(Lang::detect(Some("fr-FR")), Lang::Fr);
        assert_eq!(Lang::detect(Some("en")), Lang::En);
        assert_eq!(Lang::detect(Some("de")), Lang::En);
    }
}
