import {
  Box,
  Button,
  Card,
  CardContent,
  Collapse,
  Stack,
  Typography
} from "@mui/material";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import { useState } from "react";
import type { BriefingResponse, PredictiveNudge, RecommendedSkill } from "../types";

type Suggestion = {
  id: string;
  title: string;
  detail: string;
  type: "nudge" | "skill";
  priority: number;
  skill?: RecommendedSkill;
  nudge?: PredictiveNudge;
};

type Props = {
  briefing?: BriefingResponse;
  nudges: PredictiveNudge[];
  onExecuteSkill: (skill: RecommendedSkill) => void;
  onSnooze: (id: string) => void;
  onDismiss: (id: string) => void;
  executing: boolean;
  feedbackPending: boolean;
};

function mergeSuggestions(briefing?: BriefingResponse, nudges: PredictiveNudge[] = []): Suggestion[] {
  const items: Suggestion[] = [];

  // Nudges with recommended skills
  for (const n of nudges) {
    const skill = n.recommended_skill ||
      ((n as unknown as { recommended_action?: RecommendedSkill }).recommended_action);
    items.push({
      id: n.id,
      title: n.title,
      detail: n.detail || "",
      type: "nudge",
      priority: n.priority || 0,
      skill: skill || undefined,
      nudge: n,
    });
  }

  // Briefing recommended skills not already covered by nudges
  const nudgeIds = new Set(nudges.map((n) => n.id));
  const skills: RecommendedSkill[] =
    briefing?.recommended_skills ||
    ((briefing as unknown as { recommended_actions?: RecommendedSkill[] })?.recommended_actions || []);
  for (const s of skills) {
    if (nudgeIds.has(s.id)) continue;
    items.push({
      id: s.id,
      title: s.title,
      detail: s.summary || s.description || "",
      type: "skill",
      priority: 3,
      skill: s,
    });
  }

  // Sort by priority descending, take top 3
  items.sort((a, b) => b.priority - a.priority);
  return items.slice(0, 3);
}

export function SmartSuggestions({
  briefing,
  nudges,
  onExecuteSkill,
  onSnooze,
  onDismiss,
  executing,
  feedbackPending,
}: Props) {
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const suggestions = mergeSuggestions(briefing, nudges);

  return (
    <Card sx={{ height: "100%" }}>
      <CardContent sx={{ p: 1.55 }}>
        <Stack direction="row" alignItems="center" spacing={0.75} mb={1.25}>
          <AutoAwesomeRoundedIcon sx={{ fontSize: 18, color: "#2fd4ff" }} />
          <Box sx={{ flex: 1 }}>
            <Typography variant="h6" sx={{ fontWeight: 700 }}>
              Next Best Moves
            </Typography>
            <Typography variant="caption" color="text.secondary">
              High-confidence actions based on current tasks, memory, briefing context, and recent failures.
            </Typography>
          </Box>
        </Stack>

        {suggestions.length === 0 ? (
          <Typography variant="body2" color="text.secondary">
            No strong next moves right now.
          </Typography>
        ) : (
          <Stack spacing={0.85}>
            {suggestions.map((s, index) => (
              <Box
                key={s.id}
                className="action-row"
                sx={{
                  p: "10px 12px",
                  background: "linear-gradient(180deg, rgba(8, 18, 34, 0.76), rgba(6, 14, 28, 0.72))",
                }}
              >
                <Stack spacing={0.5}>
                  <Stack direction="row" spacing={0.75} alignItems="center" useFlexGap flexWrap="wrap">
                    <Box
                      sx={{
                        width: 22,
                        height: 22,
                        borderRadius: "50%",
                        border: "1px solid rgba(94, 184, 243, 0.35)",
                        display: "inline-flex",
                        alignItems: "center",
                        justifyContent: "center",
                        color: "rgba(144, 221, 255, 0.98)",
                        fontSize: "0.72rem",
                        fontWeight: 700,
                        flexShrink: 0,
                      }}
                    >
                      {index + 1}
                    </Box>
                    <Typography variant="body2" fontWeight={700}>
                      {s.title}
                    </Typography>
                    <Typography variant="caption" sx={{ color: "rgba(141, 192, 231, 0.72)", textTransform: "uppercase", letterSpacing: "0.08em" }}>
                      {s.type === "nudge" ? "Predicted move" : "Recommended skill"}
                    </Typography>
                    {s.nudge?.confidence ? (
                      <Typography variant="caption" sx={{ color: "rgba(132, 216, 255, 0.88)" }}>
                        {Math.round(s.nudge.confidence * 100)}% confidence
                      </Typography>
                    ) : null}
                  </Stack>
                  <Typography variant="caption" color="text.secondary" sx={{ lineHeight: 1.45 }}>
                    {s.detail.length > 100 ? s.detail.slice(0, 97) + "..." : s.detail}
                  </Typography>

                  <Collapse in={expandedId === s.id}>
                    <Box sx={{ mt: 0.5, pl: 0.5, borderLeft: "2px solid rgba(47, 212, 255, 0.3)", py: 0.5 }}>
                      <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 0.5 }}>
                        {s.detail}
                      </Typography>
                      {s.nudge?.memory_clues?.map((clue) => (
                        <Typography key={clue.id} variant="caption" color="text.secondary" sx={{ display: "block" }}>
                          {clue.memory_type} memory: {clue.summary}
                        </Typography>
                      ))}
                    </Box>
                  </Collapse>

                  <Stack direction="row" spacing={0.6} mt={0.35} useFlexGap flexWrap="wrap">
                    {s.skill ? (
                      <Button
                        variant="contained"
                        size="small"
                        disabled={executing}
                        onClick={() => onExecuteSkill(s.skill!)}
                        sx={{ textTransform: "none", minWidth: 56 }}
                      >
                        Run
                      </Button>
                    ) : null}
                    <Button
                      variant="text"
                      size="small"
                      onClick={() => setExpandedId(expandedId === s.id ? null : s.id)}
                      sx={{ textTransform: "none", minWidth: 54 }}
                    >
                      {expandedId === s.id ? "Less" : "Why?"}
                    </Button>
                    {s.type === "nudge" ? (
                      <>
                        <Button
                          variant="outlined"
                          size="small"
                          disabled={feedbackPending}
                          onClick={() => onSnooze(s.id)}
                          sx={{ textTransform: "none", minWidth: 62 }}
                        >
                          Snooze
                        </Button>
                        <Button
                          variant="text"
                          size="small"
                          color="warning"
                          disabled={feedbackPending}
                          onClick={() => onDismiss(s.id)}
                          sx={{ textTransform: "none", minWidth: 62 }}
                        >
                          Dismiss
                        </Button>
                      </>
                    ) : null}
                  </Stack>
                </Stack>
              </Box>
            ))}
          </Stack>
        )}
      </CardContent>
    </Card>
  );
}
