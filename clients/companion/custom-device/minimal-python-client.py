#!/usr/bin/env python3
"""Minimal custom AgentArk companion client.

Requires:
    pip install websockets

This example intentionally supports only pulse and command_result. Add
device-specific adapters for your own typed actions.
"""

import argparse
import asyncio
import json
import sys
import uuid
from typing import Any

import websockets


async def send_json(ws: Any, payload: dict[str, Any]) -> None:
    await ws.send(json.dumps(payload))


def command_declarations(capabilities: list[str]) -> list[dict[str, str]]:
    declarations: list[dict[str, str]] = []
    for capability in capabilities:
        action = f"{capability}.invoke"
        declarations.append(
            {
                "id": action,
                "label": action.replace(".", " ").replace("_", " ").title(),
                "capability": capability,
                "action": action,
                "description": "Custom device adapter action.",
                "risk": "high" if not capability.startswith("custom.") else "low",
            }
        )
    return declarations


async def run(args: argparse.Namespace) -> None:
    headers = {}
    if args.device_id and args.token:
        headers["Authorization"] = f"Bearer {args.token}"
        headers["X-AgentArk-Companion-Device"] = args.device_id
    async with websockets.connect(args.ws_url, additional_headers=headers) as ws:
        print(await ws.recv())
        if args.session_id and args.code:
            while True:
                await send_json(
                    ws,
                    {
                        "type": "pairing_claim",
                        "session_id": args.session_id,
                        "code": args.code,
                        "device_public_key": args.device_public_key,
                        "metadata": {"model": "minimal-python-client"},
                    },
                )
                raw = await ws.recv()
                print(raw)
                message = json.loads(raw)
                result = message.get("result") or {}
                token = result.get("device_token")
                device = result.get("device") or {}
                if token and device.get("id"):
                    print(json.dumps({"device_id": device["id"], "token": token}, indent=2))
                    return
                if result.get("status") in {"claimed", "approved"}:
                    await asyncio.sleep(3)
                    continue
                return

        if not args.device_id or not args.token:
            raise SystemExit("Provide either --session-id/--code or --device-id/--token")

        print(await ws.recv())
        await send_json(
            ws,
            {
                "type": "pulse",
                "state": "online",
                "capabilities": args.capability,
                "commands": command_declarations(args.capability),
                "metadata": {"version": "minimal-python-client"},
            },
        )

        async for raw in ws:
            message = json.loads(raw)
            print(raw)
            if message.get("type") != "command_dispatch":
                continue
            command = message.get("command") or {}
            command_id = command.get("id")
            capability = command.get("capability")
            if capability not in args.capability:
                await send_json(
                    ws,
                    {
                        "type": "command_result",
                        "command_id": command_id,
                        "success": False,
                        "error": "Capability is not supported by this custom device.",
                    },
                )
                continue
            await send_json(
                ws,
                {
                    "type": "command_result",
                    "command_id": command_id,
                    "success": True,
                    "result_preview": f"Handled {command.get('action')} for {capability}",
                },
            )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ws-url", required=True, help="wss://host/companion/ws; use ws://127.0.0.1 only for local development")
    parser.add_argument("--session-id")
    parser.add_argument("--code")
    parser.add_argument("--device-id")
    parser.add_argument("--token")
    parser.add_argument("--device-public-key", default=f"custom-python-{uuid.uuid4()}")
    parser.add_argument("--capability", action="append", default=["custom.example"])
    args = parser.parse_args()
    try:
        asyncio.run(run(args))
    except KeyboardInterrupt:
        sys.exit(130)


if __name__ == "__main__":
    main()
