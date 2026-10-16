"""Demo settings fixture: declares a config_schema and reads one value back.

It registers no tools, hooks or commands, so loading it changes no runtime behaviour; the only
observable effect is the ``demo-settings: mode=...`` line it logs, which proves the value written
by the Desktop form is what ``ctx.get_config`` returns.
"""

from __future__ import annotations

import logging

logger = logging.getLogger(__name__)


def register(ctx) -> None:
    mode = ctx.get_config("mode", default="fast")
    logger.info("demo-settings: mode=%s", mode)
