import { useEffect, useMemo, useState } from "react";
import { FileText, MessageSquare, Play } from "lucide-react";
import type { BriefingResponse, RecommendedAction } from "../../types";
import { NeuralPanel } from "./NeuralPanel";

export type SuggestedStepCardProps = {
  prompts: string[];
  onGoChat?: () => void;
  onRunBriefing?: () => void;
  briefingLoading?: boolean;
  briefing?: BriefingResponse | null;
  onExecuteAction?: (action: RecommendedAction) => void;
  executing?: boolean;
};

export function SuggestedStepCard({
  prompts,
  onGoChat,
  onRunBriefing,
  briefingLoading = false,
  briefing,
  onExecuteAction,
  executing = false,
}: SuggestedStepCardProps) {
  const heroPrompts = useMemo(() => (prompts && prompts.length > 0 ? prompts : [""]), [prompts]);
  const promptSignature = heroPrompts.join("\n");

  const [promptIndex, setPromptIndex] = useState(0);
  const [prefersReducedMotion, setPrefersReducedMotion] = useState(false);

  const activePrompt = heroPrompts[promptIndex] || heroPrompts[0] || "";
  const displayPrompt = useMemo(() => {
    const trimmed = activePrompt.trim();
    if (!trimmed) return "Ask AgentArk what needs attention next.";
    return trimmed;
  }, [activePrompt]);

  useEffect(() => {
    setPromptIndex(0);
  }, [promptSignature]);

  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) {
      return undefined;
    }

    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const syncPreference = () => {
      setPrefersReducedMotion(media.matches);
    };
    syncPreference();

    if (typeof media.addEventListener === "function") {
      media.addEventListener("change", syncPreference);
      return () => media.removeEventListener("change", syncPreference);
    }

    media.addListener(syncPreference);
    return () => media.removeListener(syncPreference);
  }, []);

  useEffect(() => {
    if (prefersReducedMotion || heroPrompts.length <= 1 || typeof window === "undefined") {
      return undefined;
    }

    const timer = window.setInterval(() => {
      setPromptIndex((prev) => (prev + 1) % heroPrompts.length);
    }, 7000);

    return () => window.clearInterval(timer);
  }, [heroPrompts.length, prefersReducedMotion]);

  const recommended = briefing?.recommended_actions ?? [];

  return (
    <NeuralPanel title="Suggested Next Step" tag="DAILY USE" tagTone="default" className="nw-panel--suggested">
      <div className="nw-suggested-prompt" title={activePrompt}>
        <span className="nw-suggested-prompt-text">{displayPrompt}</span>
      </div>

      <div className="nw-actions nw-suggested-actions">
        {onGoChat ? (
          <button className="nw-btn nw-btn--primary" type="button" onClick={onGoChat}>
            <span className="nw-btn-copy">
              <MessageSquare size={14} strokeWidth={1.9} aria-hidden />
              Ask AgentArk
            </span>
            <span className="nw-arrow">-&gt;</span>
          </button>
        ) : null}
        {onRunBriefing ? (
          <button className="nw-btn" type="button" disabled={briefingLoading} onClick={onRunBriefing}>
            <span className="nw-btn-copy">
              <FileText size={14} strokeWidth={1.9} aria-hidden />
              {briefingLoading ? "Running..." : "Generate Daily Brief"}
            </span>
            <span className="nw-arrow">-&gt;</span>
          </button>
        ) : null}
      </div>

      {recommended.length > 0 ? (
        <div className="nw-suggested-recommendations">
          {recommended.slice(0, 2).map((act) => (
            <div key={act.id} className="nw-suggested-recommendation">
              <div className="nw-suggested-run-mark">
                <Play size={11} fill="currentColor" strokeWidth={0} aria-hidden />
              </div>
              <div className="nw-suggested-recommendation-copy">
                <div className="nw-activity-ts">RECOMMENDED</div>
                <div className="nw-activity-txt">{act.title}</div>
              </div>
              {onExecuteAction ? (
                <button
                  className="nw-btn nw-btn--small nw-suggested-run"
                  type="button"
                  disabled={executing}
                  onClick={() => onExecuteAction(act)}
                >
                  Run <span className="nw-arrow">-&gt;</span>
                </button>
              ) : null}
            </div>
          ))}
        </div>
      ) : null}
    </NeuralPanel>
  );
}
