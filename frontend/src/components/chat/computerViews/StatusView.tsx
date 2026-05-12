// Calm one-line status renderer used between tool calls and as default.

import Box from "@mui/material/Box";
import Typography from "@mui/material/Typography";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import { LinkifiedText } from "./LinkifiedText";

export interface StatusViewProps {
  title: string;
  detail?: string;
}

export function StatusView({ title, detail }: StatusViewProps) {
  return (
    <Box className="cview cview-status">
      <Box className="cview-status-icon" aria-hidden="true">
        <AutoAwesomeRoundedIcon fontSize="small" />
      </Box>
      <Typography variant="subtitle1" className="cview-status-title">
        {title || "Idle"}
      </Typography>
      {detail ? (
        <Typography variant="body2" className="cview-status-detail">
          <LinkifiedText text={detail} />
        </Typography>
      ) : null}
    </Box>
  );
}

export default StatusView;
