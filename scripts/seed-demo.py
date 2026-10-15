#!/usr/bin/env python3
"""Seed neutral sessions, a team, and a group-chat mirror in isolated Hermes state."""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path


DEMO_SESSIONS = (
    (
        "launch-checklist",
        "Plan a product launch checklist",
        "Create a concise product launch checklist.",
        "The checklist is organized by preparation, launch day, and follow-up.",
    ),
    (
        "quarterly-metrics",
        "Summarize quarterly metrics",
        "Summarize the quarterly metrics for a demo review.",
        "The demo summary highlights growth, retention, and open questions.",
    ),
    (
        "billing-refactor",
        "Refactor the billing module",
        "Outline a safe refactor plan for the billing module.",
        "The plan keeps API behavior stable and adds focused verification.",
    ),
    (
        "onboarding-notes",
        "Draft onboarding notes",
        "Draft short onboarding notes for a new project.",
        "The notes cover setup, key workflows, and where to ask for help.",
    ),
    (
        "release-review",
        "Review a release plan",
        "Review a sample release plan for missing steps.",
        "The plan includes validation, staged rollout, and a rollback check.",
    ),
    (
        "research-summary",
        "Organize a research summary",
        "Organize a short research summary into clear sections.",
        "The summary separates findings, evidence, and next steps.",
    ),
)

BOT_PROFILES = (
    ("research-bot", "Research Bot", "Research and summarize neutral demo information."),
    ("writer-bot", "Writer Bot", "Draft concise neutral demo content."),
    ("reviewer-bot", "Reviewer Bot", "Review neutral demo plans for clarity."),
)
TEAM_NAME = "Demo Product Team"
ROOM_ID = "demo-product-room"
ROOM_NAME = "Demo Product Room"
GROUP_META_KEY = "hermes-bots-groups"


def seed_sessions() -> int:
    from hermes_state import SessionDB

    db = SessionDB()
    try:
        now = time.time()
        seeded = 0
        for index, (slug, title, prompt, answer) in enumerate(DEMO_SESSIONS):
            session_id = f"demo-{slug}"
            existing = db.get_session(session_id)
            if existing is None:
                db.create_session(session_id, source="desktop")
                db.set_session_title(session_id, title)
            if db.display_message_count(session_id) >= 2:
                db.reopen_session(session_id)
                continue
            db.clear_messages(session_id)
            base_time = now - (len(DEMO_SESSIONS) - index) * 60
            db.append_message(session_id, "user", prompt, observed=True, timestamp=base_time)
            db.append_message(session_id, "assistant", answer, observed=True, timestamp=base_time + 1)
            seeded += 1
        return seeded
    finally:
        db.close()


def seed_team(home: Path) -> tuple[str, list[tuple[str, str]]]:
    from hermes_cli import profiles
    from tools import bot_team

    for profile, display_name, description in BOT_PROFILES:
        if not profiles.profile_exists(profile):
            profiles.create_profile(
                profile,
                no_alias=True,
                no_skills=True,
                description=description,
            )

    team = next((item for item in bot_team.list_teams(home) if item.get("name") == TEAM_NAME), None)
    if team is None:
        team = bot_team.create_team(
            home,
            name=TEAM_NAME,
            mission="Prepare a clear, well-reviewed product launch plan.",
            actor="Demo User",
        )
    members = []
    lead_slot = None
    for profile, display_name, _description in BOT_PROFILES:
        member = bot_team.upsert_member(
            home,
            team["id"],
            profile=profile,
            role="demo teammate",
            title=display_name,
            lead=profile == "research-bot",
            **({"reports_to": lead_slot} if lead_slot and profile != "research-bot" else {}),
            actor="Demo User",
        )
        if profile == "research-bot":
            lead_slot = member["slot"]
        members.append((profile, display_name))
    return str(team["id"]), members


def seed_group_chat(home: Path, members: list[tuple[str, str]]) -> None:
    from hermes_cli.profile_ui_meta import merge_ui_meta

    now = int(time.time() * 1000)
    room = {
        "name": ROOM_NAME,
        "roomId": ROOM_ID,
        "revision": 1,
        "members": [
            {
                "connectionId": "local",
                "handle": f"@{profile}",
                "name": profile,
            }
            for profile, _display_name in members
        ],
        "log": [
            {
                "id": "demo-launch-message-1",
                "from": {"kind": "user", "name": "Demo User"},
                "text": "Let's outline the launch checklist.",
                "at": now - 180_000,
            },
            {
                "id": "demo-launch-message-2",
                "from": {"kind": "member", "name": "Research Bot"},
                "text": "Research summary is ready for the demo plan.",
                "at": now - 120_000,
            },
            {
                "id": "demo-launch-message-3",
                "from": {"kind": "member", "name": "Writer Bot"},
                "text": "I drafted the announcement and timeline.",
                "at": now - 60_000,
            },
            {
                "id": "demo-launch-message-4",
                "from": {"kind": "member", "name": "Reviewer Bot"},
                "text": "The checklist is clear and the launch risks are covered.",
                "at": now,
            },
        ],
    }
    snapshot = {
        "version": 3,
        "updatedAt": now,
        "rooms": {f"id:{ROOM_ID}": room},
    }
    result = merge_ui_meta(home, {GROUP_META_KEY: snapshot})
    if not result.get("applied"):
        raise RuntimeError("group-chat mirror could not be saved")


def main() -> int:
    home_value = os.environ.get("HERMES_HOME")
    if not home_value:
        print("seed-demo: HERMES_HOME must point to isolated state", file=sys.stderr)
        return 2
    home = Path(home_value).resolve()
    if not home.is_dir():
        print("seed-demo: isolated HERMES_HOME does not exist", file=sys.stderr)
        return 2

    sessions = seed_sessions()
    _team_id, members = seed_team(home)
    seed_group_chat(home, members)
    print(f"Seeded {len(DEMO_SESSIONS)} sessions, one team, and one group chat.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"seed-demo failed: {type(exc).__name__}", file=sys.stderr)
        raise SystemExit(1) from None
