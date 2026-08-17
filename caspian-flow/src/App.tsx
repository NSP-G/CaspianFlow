import { useEffect } from "react";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import { TitleBar } from "./components/layout/TitleBar";
import { Sidebar } from "./components/layout/Sidebar";
import { CommandPalette } from "./components/command/CommandPalette";
import { ChatPage } from "./routes/ChatPage";
import { SettingsPage } from "./routes/SettingsPage";
import { SkillsPage } from "./routes/SkillsPage";
import { KnowledgePage } from "./routes/KnowledgePage";
import { WorkflowsPage } from "./routes/WorkflowsPage";
import { WorkflowEditorPage } from "./routes/WorkflowEditorPage";
import { HelpPage } from "./routes/HelpPage";
import { HelpPanel } from "./components/help/HelpPanel";
import { OnboardingModal } from "./components/help/OnboardingModal";
import { useAppStore } from "./stores/useAppStore";
import { useCommandPalette } from "./hooks/useCommandPalette";
import { useHelp } from "./hooks/useHelp";
import { useCaspian } from "./hooks/useCaspian";
import { applyThemeDom } from "./lib/theme";

export function App() {
  const theme = useAppStore((s) => s.theme);
  const customTheme = useAppStore((s) => s.customTheme);
  const sidebarCollapsed = useAppStore((s) => s.sidebarCollapsed);
  const hasSeenOnboarding = useAppStore((s) => s.hasSeenOnboarding);
  const { open, setOpen } = useCommandPalette();
  const help = useHelp();
  const caspian = useCaspian();

  // Resolve + apply the active theme (built-in dark/light OR custom package).
  useEffect(() => {
    applyThemeDom();
  }, [theme, customTheme]);

  // On mount, restore a persisted custom theme's CSS if the store lost it
  // (e.g. page reload — only the name is persisted, not the CSS).
  useEffect(() => {
    const { customTheme: name, customThemeCss: css } = useAppStore.getState();
    if (name && !css) {
      caspian
        .getThemeCss(name)
        .then((fetched) => {
          useAppStore.getState().applyCustomTheme(name, fetched);
          applyThemeDom();
        })
        .catch(() => {
          /* theme missing on disk — silently fall back to built-in */
        });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <BrowserRouter>
      <div className="flex h-full flex-col bg-background text-foreground">
        <TitleBar />
        <div className="flex min-h-0 flex-1">
          <Sidebar collapsed={sidebarCollapsed} />
          <main className="flex min-w-0 flex-1 flex-col">
            <Routes>
              <Route path="/" element={<ChatPage />} />
              <Route path="/skills" element={<SkillsPage />} />
              <Route path="/knowledge" element={<KnowledgePage />} />
              <Route path="/workflows" element={<WorkflowsPage />} />
              <Route path="/workflows/:name" element={<WorkflowEditorPage />} />
              <Route path="/settings" element={<SettingsPage />} />
              <Route path="/help" element={<HelpPage />} />
            </Routes>
          </main>
        </div>
      </div>
      <CommandPalette open={open} onOpenChange={setOpen} />
      <HelpPanel open={help.open} onClose={() => help.setOpen(false)} />
      {!hasSeenOnboarding && <OnboardingModal />}
    </BrowserRouter>
  );
}
