import { Box, Button, Stack, Typography } from "@mui/material";
import { useQueryClient } from "@tanstack/react-query";
import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import { SettingsOverviewTab } from "./SettingsOverviewTab";
import {
  getSettingsPageMeta,
  normalizeSettingsTab,
  resolveInitialSettingsTab,
  type SettingsPageProps,
} from "./settingsMeta";
import {
  getSelectedSettingsNav,
  SettingsNavigation,
} from "./settingsNavigation";
import { prefetchSettingsPageData } from "./settingsData";
import {
  preloadCommonSettingsPanels,
  preloadSettingsTab,
} from "./workspacePreload";

const SettingsPageFull = lazy(() => import("./SettingsPageFull"));

function SettingsFallback() {
  return (
    <Box className="list-shell" sx={{ minHeight: 180, p: 1.5 }}>
      <Typography variant="body2" sx={{ color: "text.secondary" }}>
        Loading settings...
      </Typography>
    </Box>
  );
}

export default function SettingsPage({
  autoRefresh,
  initialTab,
  hideSettingsNav,
  standaloneSurface,
}: SettingsPageProps) {
  const queryClient = useQueryClient();
  const entryTab = resolveInitialSettingsTab(initialTab);
  const [tab, setTab] = useState(() => entryTab);
  const [fullEditorOpen, setFullEditorOpen] = useState(
    () => Boolean(standaloneSurface) || entryTab !== 0,
  );
  const selectedSettingsMeta = useMemo(() => getSettingsPageMeta(tab), [tab]);

  useEffect(() => {
    const nextTab = resolveInitialSettingsTab(initialTab);
    setTab((current) => (current === nextTab ? current : nextTab));
    setFullEditorOpen(Boolean(standaloneSurface) || nextTab !== 0);
  }, [initialTab, standaloneSurface]);

  useEffect(() => {
    if (standaloneSurface === "arkpulse") return;
    preloadCommonSettingsPanels();
    prefetchSettingsPageData(queryClient);
  }, [queryClient, standaloneSurface]);

  const openFullEditor = () => {
    preloadCommonSettingsPanels();
    prefetchSettingsPageData(queryClient);
    setFullEditorOpen(true);
  };

  const changeTab = (nextTabRaw: number) => {
    const nextTab = normalizeSettingsTab(nextTabRaw);
    preloadSettingsTab(nextTab);
    if (nextTab !== 0) {
      prefetchSettingsPageData(queryClient);
    }
    setTab(nextTab);
    setFullEditorOpen(nextTab !== 0);
  };

  if (standaloneSurface || fullEditorOpen || tab !== 0) {
    return (
      <Suspense fallback={<SettingsFallback />}>
        <SettingsPageFull
          autoRefresh={autoRefresh}
          initialTab={tab}
          hideSettingsNav={hideSettingsNav}
          standaloneSurface={standaloneSurface}
        />
      </Suspense>
    );
  }

  const selectedSettingsNav = getSelectedSettingsNav(tab, 0);
  const selectedSettingsHeaderTitle =
    selectedSettingsMeta.title || selectedSettingsNav?.label || "Settings";

  return (
    <WorkspacePageShell spacing={1.35}>
      <Box
        className="settings-shell-layout"
        sx={{
          flex: 1,
          minHeight: 0,
          ...(hideSettingsNav
            ? { gridTemplateColumns: "1fr !important" }
            : undefined),
        }}
      >
        {!hideSettingsNav ? (
          <SettingsNavigation tab={tab} onTabChange={changeTab} />
        ) : null}
        <Box
          className={`settings-main${hideSettingsNav ? " settings-main-standalone" : ""}`}
        >
          <Stack spacing={2} className="workspace-page-shell settings-page-shell">
            <WorkspacePageHeader
              eyebrow={selectedSettingsMeta.kicker}
              title={selectedSettingsHeaderTitle}
              description={selectedSettingsMeta.description}
              className="settings-page-header"
              actions={
                <Button size="small" variant="contained" onClick={openFullEditor}>
                  Edit setup
                </Button>
              }
            />
            <SettingsOverviewTab
              autoRefresh={autoRefresh}
              onOpenFullSettings={openFullEditor}
              onNavigateTab={changeTab}
            />
          </Stack>
        </Box>
      </Box>
    </WorkspacePageShell>
  );
}
