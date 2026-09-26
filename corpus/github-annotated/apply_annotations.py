#!/usr/bin/env python3
"""Generate the hand-annotated .prompt.rtpl files in this directory from corpus/github/*.md.

This is the provenance record for corpus/github-annotated/. Every annotation is expressed
as an insertion at a 1-based source line number, so the original prompt text is preserved
byte-for-byte: removing every inserted line from an annotated file reproduces its source
exactly (asserted on every build; see ANNOTATION_NOTES.md section 2).

Run from the repository root:

    python3 corpus/github-annotated/apply_annotations.py            # rebuild + stats
    python3 corpus/github-annotated/apply_annotations.py --review   # show each insertion
                                                                    # next to the line it
                                                                    # annotates

The annotation criteria (category vocabulary, priority convention, and the four conditions
under which a natural-language conditional was lifted into {% if %}) are documented in
ANNOTATION_NOTES.md.
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(HERE, os.pardir, "github")
OUT = HERE

# spec: file -> list of (lineno_1based, position, [lines])
# position: "before" or "after"
SPEC = {}


def R(n, *lines):
    return (n, "before", list(lines))


def A(n, *lines):
    return (n, "after", list(lines))


# ---------------------------------------------------------------- F1
SPEC["09e319ab5af3_skill.md"] = [
    R(9, "// @rule RULE:GET_WEATHER_TOOL category=process"),
    R(14, "// @rule RULE:WEATHER_RETURN_SHAPE category=format"),
    R(23, "// @rule RULE:ERROR_ON_LOOKUP_FAILURE category=format"),
]

# ---------------------------------------------------------------- F2
SPEC["194e3e6b9421_SKILL.md"] = [
    R(15, "// @rule RULE:RUN_PREAMBLE_FIRST category=process priority=20"),
    R(26, "// @rule RULE:UPGRADE_PROMPT_FLOW category=process"),
    R(33, "// @rule RULE:JUST_UPGRADED_NOTICE category=format"),
    R(42, "// @rule RULE:PIPELINE_SEQUENCE category=process"),
    R(63, "// @rule RULE:GO_ENTRYPOINT category=process"),
    R(68, "// @rule RULE:ROUTE_DO_MARKETING category=process"),
    R(69, "// @rule RULE:ROUTE_SHIP category=process"),
    R(70, "// @rule RULE:ROUTE_FIRST_TIME category=process"),
    R(71, "// @rule RULE:ROUTE_HAS_STRATEGY category=process"),
    R(72, "// @rule RULE:ROUTE_SPECIFIC_QUESTION category=process"),
    R(73, "// @rule RULE:ROUTE_UPGRADE category=process"),
]

# ---------------------------------------------------------------- F3
SPEC["7a8c0a5fe49b_SKILL.md"] = [
    R(13, "// @rule RULE:CATEGORY_SET category=policy"),
    R(17, "// @rule RULE:RUN_FROM_PROJECT_DIR category=process"),
    R(56, "// @rule RULE:OUTPUT_STREAMS category=format"),
    R(70, "// @rule RULE:PIPELINE_STAGES category=process"),
    R(90, "// @rule RULE:STATUS_FIRST category=process priority=20"),
    R(91, "// @rule RULE:DRY_RUN_PREVIEW category=process"),
    R(93, "// @rule RULE:RECLASSIFY_ON_ERROR category=process"),
]

# ---------------------------------------------------------------- F4  bellwether
SPEC["541a1bc0d709_SKILL.md"] = [
    R(14, "// @rule RULE:LOOP_SHAPE category=process"),
    R(16, "// @rule RULE:NO_SUBAGENTS category=policy priority=30"),
    R(20, "// @rule RULE:CLI_ONLY_STATUS category=policy priority=30"),
    R(21, "// @rule RULE:PENDING_NOT_SUCCESS category=policy priority=30"),
    R(22, "// @rule RULE:NO_MANUAL_GH_API category=policy priority=30"),
    R(23, "// @rule RULE:NO_SLEEP category=policy priority=30"),
    R(24, "// @rule RULE:NO_MANUAL_REVIEW_PARSE category=policy priority=30"),
    R(25, "// @rule RULE:INSTALL_IF_MISSING category=process priority=30"),
    R(26, "// @rule RULE:ALL_VIA_CLI category=policy priority=30"),
    R(30, "// @rule RULE:DECISION_TABLE_FIRST_MATCH category=process priority=30"),
    R(34, "// @rule RULE:DT1_FIX_CI category=process priority=30"),
    R(35, "// @rule RULE:DT2_FIX_REVIEWS category=process priority=30"),
    R(36, "// @rule RULE:DT3_SYNC_BRANCH category=process priority=30"),
    R(37, "// @rule RULE:DT4_RESTART_WATCH category=process priority=30"),
    R(38, "// @rule RULE:DT5_READY category=process priority=30"),
    R(39, "// @rule RULE:DT6_MERGED_CLOSED category=process priority=30"),
    R(40, "// @rule RULE:DT7_AWAIT_HUMAN category=process priority=30"),
    R(41, "// @rule RULE:DT8_INFRA_ONLY category=process priority=30"),
    R(43, "// @rule RULE:NO_STATUS_ONLY category=policy priority=30"),
    R(45, "// @rule RULE:REVIEWS_DESPITE_PENDING category=process"),
    R(47, "// @rule RULE:ACT_IMMEDIATELY category=process"),
    R(49, "// @rule RULE:REVIEWS_IN_SCOPE category=policy"),
    R(53, "// @rule RULE:LOOP_PROCEDURE category=process"),
    R(64, "// @rule RULE:PENDING_NEVER_TERMINAL category=policy"),
    R(74, "// @rule RULE:SUCCESS_CONDITION category=policy priority=30"),
    R(78, "// @rule RULE:SKIP_INFRA category=policy"),
    R(82, "// @rule RULE:READ_ERROR_LOG category=process"),
    R(83, "// @rule RULE:REPRO_LOCALLY category=process priority=20"),
    R(84, "// @rule RULE:ROOT_CAUSE_FIRST category=process priority=20"),
    R(86, "// @rule RULE:FIX_PATTERN_EVERYWHERE category=process"),
    R(87, "// @rule RULE:ADD_REGRESSION_TEST category=process"),
    R(88, "// @rule RULE:NO_SUPPRESSIONS category=policy"),
    R(89, "// @rule RULE:MINIMAL_FIX category=process"),
    R(90, "// @rule RULE:RERUN_UNTIL_PASS category=process"),
    R(91, "// @rule RULE:STAGE_BY_NAME category=policy"),
    R(92, "// @rule RULE:RESTART_WATCH_AFTER_PUSH category=process"),
    R(94, "// @rule RULE:CI_GREEN_BEFORE_REVIEWS category=process priority=30",
        "// @breakpoint BREAKPOINT:CI_GREEN_GATE"),
    R(98, "// @rule RULE:BATCH_COMMENTS category=process priority=20"),
    R(102, "// @rule RULE:STALENESS_FIRST category=process priority=20"),
    R(103, "// @rule RULE:STALE_CHECK_FILE category=process"),
    R(104, "// @rule RULE:STALE_CODE_CHANGED category=process"),
    R(105, "// @rule RULE:STALE_ALREADY_IMPLEMENTED category=process"),
    R(106, "// @rule RULE:STALE_ACT_ONLY_IF_APPLIES category=policy"),
    R(108, "// @rule RULE:CLASSIFY_ALL_REVIEWS category=process"),
    R(110, "// @rule RULE:BOT_COMMENT_TRIAGE category=policy"),
    R(113, "// @rule RULE:DEFAULT_TO_FIXING category=policy"),
    R(115, "// @rule RULE:BOT_SEVERITY_HEURISTICS category=policy"),
    R(121, "// @rule RULE:HUMAN_COMMENT_TRIAGE category=policy"),
    R(128, "// @rule RULE:FIX_AND_COMMIT category=process"),
    R(130, "// @rule RULE:COMMIT_BEFORE_REPLY category=process priority=30",
        "// @breakpoint BREAKPOINT:BEFORE_REPLIES"),
    R(134, "// @rule RULE:REPLY_AND_RESOLVE category=policy priority=30"),
    R(136, "// @rule RULE:ALWAYS_RESOLVE category=policy priority=30"),
    R(138, "// @rule RULE:ONE_REPLY_PER_COMMENT category=process"),
    R(152, "// @rule RULE:RESOLVE_NO_EXCEPTIONS category=policy priority=30"),
    R(154, "// @rule RULE:VERIFY_AFTER_REPLIES category=process"),
    R(155, "// @rule RULE:RERESOLVE_IF_ACTIONABLE category=process"),
    R(157, "// @rule RULE:WATCH_AFTER_RESOLVE category=process priority=30"),
    R(161, "// @rule RULE:FIX_EVERYTHING_DONT_ASK category=policy priority=30"),
    R(162, "// @rule RULE:NEVER_PAUSE category=policy priority=30"),
    R(163, "// @rule RULE:IN_PROGRESS_STAY_IN_LOOP category=process"),
    R(164, "// @rule RULE:REACT_FIRST_SIGNAL category=process"),
    R(165, "// @rule RULE:FIX_NEEDS_LOCAL_PASS category=process"),
    R(166, "// @rule RULE:BACKLOG_IS_WORK category=policy"),
    R(167, "// @rule RULE:ONE_FIX_PER_CYCLE category=process"),
    R(168, "// @rule RULE:MINIMAL_CHANGES category=process"),
    R(169, "// @rule RULE:EVERY_THREAD_RESOLVED category=policy priority=30"),
    R(170, "// @rule RULE:VERIFY_BEFORE_PUSHING category=process"),
    R(171, "// @rule RULE:NEVER_STOP_UNTIL_TERMINAL category=policy priority=30"),
    R(172, "// @rule RULE:RECOGNIZE_TERMINAL_BLOCKERS category=process"),
    R(173, "// @rule RULE:NO_SUBAGENT_PRINCIPLE category=policy"),
]

# ---------------------------------------------------------------- F5  apple health
SPEC["23b1dabda63a_SKILL.md"] = [
    R(20, "// @rule RULE:REPORT_STANDARD category=policy"),
    R(22, "// @rule RULE:MEDICAL_DISCLAIMER category=safety"),
    R(28, "// @rule RULE:LOCATE_EXPORT category=process"),
    R(44, "// @rule RULE:WARN_LARGE_EXPORT category=process"),
    R(59, "// @rule RULE:READ_DATA_QUALITY_FIRST category=process priority=20",
        "// @breakpoint BREAKPOINT:BEFORE_INTERPRETATION"),
    R(62, "// @rule RULE:SKIP_INSUFFICIENT category=policy"),
    R(63, "// @rule RULE:CAVEAT_LOW category=format"),
    R(64, "// @rule RULE:NOTE_MODERATE category=format"),
    R(65, "// @rule RULE:PRESENT_HIGH category=format"),
    R(67, "// @rule RULE:CHECK_DATA_REQUIREMENTS category=process"),
    R(69, "// @rule RULE:USE_CLINICAL_REFERENCES category=policy"),
    R(73, "// @rule RULE:FILL_NARRATIVES category=process"),
    R(95, "// @rule RULE:NARRATIVE_HTML category=format"),
    R(96, "// @rule RULE:NARRATIVE_CITE_NUMBERS category=format"),
    R(97, "// @rule RULE:NARRATIVE_CITE_RANGES category=policy"),
    R(98, "// @rule RULE:NARRATIVE_EVIDENCE_GRADES category=format"),
    R(99, "// @rule RULE:NARRATIVE_USER_LANGUAGE category=format"),
    R(100, "// @rule RULE:NARRATIVE_TONE category=style"),
    R(119, "// @rule RULE:NEVER_SIMPLIFY category=policy priority=30"),
    R(125, "// @rule RULE:NO_FABRICATION category=safety priority=30"),
    R(126, "// @rule RULE:STATE_MISSING category=format"),
    R(127, "// @rule RULE:EXPLAIN_MISSING_IMPACT category=format"),
    R(128, "// @rule RULE:ADJUST_SCORES category=policy"),
    R(132, "// @rule RULE:EVIDENCE_GRADE_REQUIRED category=format"),
    R(145, "// @rule RULE:MATCH_USER_LANGUAGE category=format priority=30"),
    R(150, "// @rule RULE:PASS_LANG_FLAG category=process"),
    R(156, "// @rule RULE:CHINESE_TERMINOLOGY category=format"),
    R(170, "// @rule RULE:READ_REFERENCE_FILES category=process"),
]

# ---------------------------------------------------------------- F6  claude-to-im
SPEC["8ad9c110c8fc_SKILL.md"] = [
    R(25, "// @rule RULE:ROLE category=style"),
    R(30, "// @rule RULE:LOCATE_SKILL_DIR category=process"),
    R(34, "// @rule RULE:PARSE_SUBCOMMAND category=process"),
    R(47, "// @rule RULE:STATUS_VS_DOCTOR category=policy"),
    R(49, "// @rule RULE:LOGS_ARG_DEFAULT category=process"),
    R(51, "// @rule RULE:READ_SETUP_GUIDE_FIRST category=process priority=20"),
    R(55, "// @rule RULE:DETECT_RUNTIME category=process priority=20"),
    # --- branch 1: runtime
    R(57, "// @condition CONDITION:RUNTIME",
        "{% if runtime.claude_code %}",
        "// @rule RULE:RUNTIME_CLAUDE_CODE category=process"),
    R(58, "{% else %}",
        "// @rule RULE:RUNTIME_CODEX_FALLBACK category=process priority=10"),
    A(58, "{% endif %}"),
    R(60, "// @rule RULE:RUNTIME_SELF_TEST category=process"),
    R(64, "// @rule RULE:CONFIG_PRECHECK category=process priority=20"),
    # --- branch 2: config present
    R(66, "// @condition CONDITION:CONFIG_PRESENT",
        "{% if not config.exists %}"),
    R(67, "// @rule RULE:NO_CONFIG_CLAUDE_CODE category=process"),
    R(68, "// @rule RULE:NO_CONFIG_CODEX category=process"),
    R(69, "{% else %}",
        "// @rule RULE:CONFIG_PRESENT_PROCEED category=process"),
    A(69, "{% endif %}"),
    R(75, "// @rule RULE:SETUP_WIZARD category=process"),
    R(77, "// @rule RULE:ONE_FIELD_AT_A_TIME category=process"),
    R(81, "// @rule RULE:ASK_CHANNELS category=process"),
    R(91, "// @rule RULE:COLLECT_CREDENTIALS category=process"),
    R(93, "// @rule RULE:TELEGRAM_CREDENTIALS category=policy"),
    R(94, "// @rule RULE:DISCORD_CREDENTIALS category=policy"),
    R(95, "// @rule RULE:FEISHU_CREDENTIALS category=policy"),
    R(100, "// @rule RULE:QQ_CREDENTIALS category=policy"),
    R(108, "// @rule RULE:WEIXIN_QR_LOGIN category=policy"),
    R(116, "// @rule RULE:DINGTALK_CREDENTIALS category=policy"),
    R(124, "// @rule RULE:ASK_GENERAL_SETTINGS category=process"),
    R(130, "// @rule RULE:NO_HARDCODED_MODEL category=policy priority=30"),
    R(135, "// @rule RULE:SHOW_SUMMARY_MASKED category=format"),
    R(136, "// @rule RULE:CONFIRM_BEFORE_WRITE category=process priority=20",
        "// @breakpoint BREAKPOINT:BEFORE_CONFIG_WRITE"),
    R(139, "// @rule RULE:CHMOD_CONFIG category=safety"),
    R(140, "// @rule RULE:VALIDATE_TOKENS category=process"),
    R(146, "// @rule RULE:START_PRECHECK category=process priority=20"),
    R(150, "// @rule RULE:START_FAILURE_GUIDANCE category=process"),
    R(164, "// @rule RULE:LOGS_DEFAULT_N category=process"),
    R(169, "// @rule RULE:HANDOFF_PURPOSE category=process"),
    R(183, "// @rule RULE:HANDOFF_CODEX_THREAD category=process"),
    R(184, "// @rule RULE:HANDOFF_DETECT_SESSION category=process priority=10"),
    R(186, "// @rule RULE:HANDOFF_NO_BINDING category=process"),
    R(187, "// @rule RULE:HANDOFF_MULTIPLE_BINDINGS category=policy"),
    R(193, "// @rule RULE:PROXY_IN_CONFIG category=process"),
    R(201, "// @rule RULE:REMOVED_COMMANDS category=policy"),
    R(203, "// @rule RULE:RESUME_LIMITATIONS_NOTICE category=safety"),
    R(207, "// @rule RULE:RECONFIGURE_FLOW category=process"),
    R(219, "// @rule RULE:RUN_DOCTOR category=process"),
    R(221, "// @rule RULE:DOCTOR_COMMON_FIXES category=process"),
    R(228, "// @rule RULE:READ_TROUBLESHOOTING category=process"),
    R(230, "// @rule RULE:FEISHU_UPGRADE_NOTE category=process"),
    R(234, "// @rule RULE:MASK_SECRETS category=safety priority=30"),
    R(235, "// @rule RULE:ALWAYS_CHECK_CONFIG category=policy priority=30"),
]

# ---------------------------------------------------------------- F7  mansplain
SPEC["728fe4446723_SKILL.md"] = [
    R(19, "// @rule RULE:WHEN_TO_USE category=process"),
    R(28, "// @rule RULE:GATHER_CONTEXT category=process"),
    R(32, "// @rule RULE:DETERMINE_SECTION category=process"),
    R(33, "// @rule RULE:WRITE_MDOC category=format"),
    R(34, "// @rule RULE:VALIDATE_MANDOC category=process"),
    R(35, "// @rule RULE:VALIDATE_MANSPLAIN category=process"),
    R(36, "// @rule RULE:FILE_PLACEMENT category=process"),
    # --- branch: mansplain CLI installed
    R(38, "// @condition CONDITION:MANSPLAIN_CLI",
        "{% if env.mansplain_cli %}"),
    R(40, "// @rule RULE:RONN_ALTERNATIVE category=process priority=10"),
    R(47, "// @rule RULE:PREFER_MDOC_FOR_INITIAL category=policy priority=20"),
    A(49, "{% endif %}"),
    R(52, "// @rule RULE:OUTPUT_MDOC_ONLY category=format"),
    R(56, "// @rule RULE:HEADER_REQUIRED category=format"),
    R(64, "// @rule RULE:HEADER_MACROS category=format"),
    R(68, "// @rule RULE:REQUIRED_SECTIONS category=format priority=20"),
    R(93, "// @rule RULE:MATCH_EXAMPLE category=format"),
    R(166, "// @rule RULE:NO_RAW_TROFF category=policy priority=30"),
    R(185, "// @rule RULE:SECTION_ORDER category=format"),
    R(188, "// @rule RULE:MINIMUM_SECTIONS category=format"),
    R(192, "// @rule RULE:BE_TERSE category=style"),
    R(193, "// @rule RULE:EVERY_FLAG_DOCUMENTED category=format"),
    R(194, "// @rule RULE:EXAMPLES_REALISTIC category=format"),
    R(195, "// @rule RULE:LITERAL_BLOCKS category=format"),
    R(196, "// @rule RULE:SUBCOMMAND_MACRO category=format"),
    R(197, "// @rule RULE:TITLE_UPPERCASE category=format"),
    R(201, "// @rule RULE:SECTION1_GUIDANCE category=format"),
    R(203, "// @rule RULE:SECTION5_GUIDANCE category=format"),
    R(209, "// @rule RULE:SECTION7_GUIDANCE category=format"),
    R(229, "// @rule RULE:SECTION_CHOICE category=process"),
    R(234, "// @rule RULE:VALIDATE_AFTER_GENERATION category=process priority=20"),
    R(249, "// @rule RULE:MAN_DIR category=process"),
]

# ---------------------------------------------------------------- F8  ai-emergency-tools
SPEC["053136ecb86a_SKILL.md"] = [
    R(20, "// @rule RULE:VT_API_KEY_CONFIG category=process"),
    R(39, "// @rule RULE:NO_COMMIT_API_KEY category=safety priority=30"),
    R(40, "// @rule RULE:NO_SHARE_API_KEY category=safety priority=30"),
    R(41, "// @rule RULE:ROTATE_API_KEY category=safety priority=30"),
    R(45, "// @rule RULE:SSH_INFO_REQUIRED category=process"),
    R(55, "// @rule RULE:USE_SSH_KEY category=safety"),
    R(56, "// @rule RULE:DEDICATED_ACCOUNT category=safety"),
    R(57, "// @rule RULE:ROTATE_PASSWORD category=safety"),
    R(72, "// @rule RULE:SUBMODULE_TRIGGERS category=process"),
    R(83, "// @rule RULE:INVOKE_SHELLCODE_ANALYZE category=process"),
    R(96, "// @rule RULE:SHELLCODE_PARAMS category=process"),
    R(103, "// @rule RULE:INVOKE_LINUX_ER category=process"),
    R(117, "// @rule RULE:INVESTIGATION_DIRECTIONS category=process"),
    R(133, "// @rule RULE:ROUTING_LOGIC category=process"),
    R(151, "// @rule RULE:ASK_WHEN_AMBIGUOUS category=process priority=10"),
    R(209, "// @rule RULE:THREAT_LEVELS category=format"),
    R(221, "// @rule RULE:REPORT_TEMPLATE category=format"),
    R(283, "// @rule RULE:AUTHORIZED_USE_ONLY category=safety priority=30"),
    R(284, "// @rule RULE:ISOLATED_ENVIRONMENT category=safety priority=30"),
    R(285, "// @rule RULE:PROTECT_RESULTS category=safety priority=30"),
    R(286, "// @rule RULE:LEGAL_COMPLIANCE category=safety priority=30"),
    R(287, "// @rule RULE:NO_DESTRUCTIVE_ON_PROD category=safety priority=30"),
]

# ---------------------------------------------------------------- F9  research-visualizer
SPEC["ab8bd009141a_SKILL.md"] = [
    R(40, "// @rule RULE:SET_SHELL_VARS category=process"),
    R(47, "// @rule RULE:GEN_PRECHECK category=process priority=20",
        "// @breakpoint BREAKPOINT:GEN_PRECHECK"),
    # --- branch A: no-topic invocation
    R(55, "// @condition CONDITION:NO_TOPIC",
        "{% if not invocation.topic %}"),
    R(57, "// @rule RULE:NO_TOPIC_STOP category=process priority=30"),
    A(57, "{% endif %}"),
    R(67, "// @rule RULE:CHECK_POINTER_CONFIG category=process"),
    # --- branch B: hub config exists
    R(68, "// @condition CONDITION:HUB_CONFIG",
        "{% if hub.config_exists %}",
        "// @rule RULE:HUB_EXISTS_FLOW category=process"),
    R(73, "{% else %}",
        "// @rule RULE:FIRST_TIME_SETUP category=process priority=10"),
    A(73, "{% endif %}"),
    R(77, "// @rule RULE:NO_HARD_DEPENDENCIES category=policy"),
    R(85, "// @rule RULE:TIMING_DEFAULT_ON category=policy"),
    # --- branch C: phase timing enabled
    R(91, "// @condition CONDITION:PHASE_TIMING",
        "{% if not timing.enabled %}",
        "// @rule RULE:TIMING_DISABLED_SKIP category=process"),
    R(93, "{% else %}"),
    R(94, "// @rule RULE:TRACK_PHASE_BOUNDARIES category=process"),
    R(96, "// @rule RULE:ONE_COMMAND_PER_TRACK category=process priority=30"),
    R(104, "// @rule RULE:SAFE_TO_AUTORUN category=process"),
    R(105, "// @rule RULE:ALL_NINE_PHASES category=process"),
    A(105, "{% endif %}"),
    R(115, "// @rule RULE:PARSE_INTENT category=process"),
    R(124, "// @rule RULE:EXTENSION_DETECTION category=process"),
    R(138, "// @rule RULE:BROAD_SEARCHES category=process"),
    R(140, "// @rule RULE:TEMPORAL_SEGMENTATION category=process"),
    R(155, "// @rule RULE:SPLIT_TEST category=policy"),
    R(178, "// @rule RULE:CHECKPOINT_ASK category=process",
        "// @breakpoint BREAKPOINT:USER_CHECKPOINT"),
    R(180, "// @rule RULE:WAIT_FOR_APPROVAL category=policy priority=30"),
    R(182, "// @rule RULE:VISIBILITY_AND_CONSENT category=safety priority=30"),
    R(194, "// @rule RULE:GATHER_PER_CELL category=process"),
    R(195, "// @rule RULE:TAG_QUALITY_TIER category=format"),
    R(196, "// @rule RULE:TRIANGULATE category=policy"),
    R(197, "// @rule RULE:MARK_ESTIMATES category=safety"),
    R(199, "// @rule RULE:TRACK_SOURCES category=format"),
    R(213, "// @rule RULE:DECIDE_AFTER_DATA category=process priority=20"),
    R(231, "// @rule RULE:NO_STANDALONE_VITE category=policy priority=30"),
    R(235, "// @rule RULE:GEN_HANDLES_STRUCTURE category=policy"),
    R(251, "// @rule RULE:CODE_HYGIENE category=policy"),
    R(253, "// @rule RULE:ATOMIC_WRITE_GROUPS category=process priority=20"),
    R(266, "// @rule RULE:IMPORT_PATH category=format"),
    R(280, "// @rule RULE:STORE_QUERY category=process"),
    R(286, "// @rule RULE:GLOSSARY_ENRICH category=process"),
    R(288, "// @rule RULE:GLOSSARY_DENSITY category=format"),
    R(289, "// @rule RULE:GLOSSARY_PRIORITY category=policy"),
    R(300, "// @rule RULE:GLOSSARY_JSX_ONLY category=policy priority=30"),
    R(301, "// @rule RULE:GLOSSARY_KEY_MATCH category=policy"),
    R(302, "// @rule RULE:GLOSSARY_SINGLE_PASS category=process"),
    R(303, "// @rule RULE:NO_INVENTED_PROPS category=policy"),
    R(315, "// @rule RULE:PRESENT_STEP_ORDER category=process priority=30",
        "// @breakpoint BREAKPOINT:BEFORE_DELIVERY"),
    R(317, "// @rule RULE:RUN_VALIDATE category=process"),
    R(318, "// @rule RULE:ZERO_WARNING_GATE category=policy priority=30"),
    R(320, "// @rule RULE:QA_CHECKS category=process"),
    R(326, "// @rule RULE:TELEMETRY_GATE category=process"),
    R(327, "// @rule RULE:MODEL_FIELD category=format"),
    R(330, "// @rule RULE:GIT_SYNC category=process"),
    R(331, "// @rule RULE:LIBRARY_SHARE category=safety"),
    R(333, "// @rule RULE:DELIVER category=process"),
]

# ---------------------------------------------------------------- F10  warren-buffett
SPEC["b1950a5dde10_SKILL.md"] = [
    R(17, "// @rule RULE:ROLE_PLAY category=style priority=30"),
    R(19, "// @rule RULE:FIRST_PERSON category=style priority=30"),
    R(20, "// @rule RULE:TONE category=style priority=30"),
    R(21, "// @rule RULE:USE_ANALOGIES category=style priority=30"),
    R(22, "// @rule RULE:SELF_DEPRECATING_HUMOR category=style priority=30"),
    R(23, "// @rule RULE:LETTER_STYLE category=style priority=30"),
    R(25, "// @rule RULE:DISCLAIMER category=safety priority=30"),
    R(28, "// @rule RULE:EXIT_ROLE category=policy priority=30"),
    R(36, "// @rule RULE:CLASSIFY_QUESTION category=process priority=20"),
    # --- branch: company analysis only
    R(44, "// @condition CONDITION:COMPANY_ANALYSIS",
        '{% if question.type == "company" %}'),
    R(46, "// @rule RULE:DATA_FIRST category=policy priority=30"),
    R(50, "// @rule RULE:ASK_USER_FOR_DATA category=process priority=20"),
    R(57, "// @rule RULE:WEBSEARCH_FALLBACK category=process priority=10"),
    R(63, "// @rule RULE:APPLY_FRAMEWORK category=process"),
    R(71, "// @rule RULE:NO_RESEARCH_DUMP category=format"),
    A(77, "{% endif %}"),
    R(81, "// @rule RULE:ANSWER_FROM_MODELS category=process"),
    R(82, "// @rule RULE:LEAD_WITH_VERDICT category=format priority=20"),
    R(83, "// @rule RULE:EXPLAIN_BY_ANALOGY category=style"),
    R(84, "// @rule RULE:TELL_A_STORY category=style"),
    R(85, "// @rule RULE:GIVE_CLEAR_ADVICE category=format"),
    R(86, "// @rule RULE:ADMIT_OUT_OF_SCOPE category=safety"),
    R(106, "// @rule RULE:MOAT_TEST category=policy"),
    R(119, "// @rule RULE:MR_MARKET_APPLICATION category=policy"),
    R(135, "// @rule RULE:COMPETENCE_APPLICATION category=policy"),
    R(151, "// @rule RULE:MARGIN_APPLICATION category=policy"),
    R(167, "// @rule RULE:COMPOUNDING_APPLICATION category=policy"),
    R(180, "// @rule RULE:HEURISTIC_ONE_SENTENCE category=policy"),
    R(191, "// @rule RULE:HEURISTIC_TEN_YEARS category=policy"),
    R(202, "// @rule RULE:HEURISTIC_HUNDRED_MILLION category=policy"),
    R(213, "// @rule RULE:HEURISTIC_ROE category=policy"),
    R(224, "// @rule RULE:HEURISTIC_FCF category=policy"),
    R(235, "// @rule RULE:HEURISTIC_MANAGEMENT category=policy"),
    R(248, "// @rule RULE:HEURISTIC_COMPETENCE category=policy"),
    R(257, "// @rule RULE:HEURISTIC_MARGIN category=policy"),
    R(271, "// @rule RULE:SHORT_SENTENCES category=style"),
    R(272, "// @rule RULE:RHETORICAL_QUESTIONS category=style"),
    R(273, "// @rule RULE:CONTRAST_CONSTRUCTIONS category=style"),
    R(274, "// @rule RULE:QUOTE_MUNGER category=style"),
    R(277, "// @rule RULE:PREFERRED_VOCAB category=style"),
    R(279, "// @rule RULE:FORBIDDEN_JARGON category=style"),
    R(282, "// @rule RULE:ANSWER_RHYTHM category=format"),
    R(289, "// @rule RULE:ANALOGY_SOURCES category=style"),
    R(303, "// @rule RULE:SELF_MOCKERY_EXAMPLES category=style"),
    R(325, "// @rule RULE:PROSE_STYLE category=style"),
    R(330, "// @rule RULE:PLAIN_TERMS category=style"),
    R(341, "// @rule RULE:VALUE_LONG_TERM category=policy priority=50"),
    R(342, "// @rule RULE:VALUE_HONESTY category=policy priority=40"),
    R(343, "// @rule RULE:VALUE_COMPETENCE category=policy priority=30"),
    R(344, "// @rule RULE:VALUE_RATIONALITY category=policy priority=20"),
    R(345, "// @rule RULE:VALUE_SIMPLICITY category=policy priority=10"),
    R(349, "// @rule RULE:REJECT_SPECULATION category=policy"),
    R(350, "// @rule RULE:REJECT_LEVERAGE category=policy"),
    R(351, "// @rule RULE:REJECT_COMPLEX_INSTRUMENTS category=policy"),
    R(352, "// @rule RULE:REJECT_HYPE category=policy"),
    R(353, "// @rule RULE:REJECT_OVER_DIVERSIFICATION category=policy"),
    R(366, "// @rule RULE:DATA_PRIORITY_USER category=process priority=30"),
    R(376, "// @rule RULE:DATA_PRIORITY_SEARCH category=process priority=20"),
    R(391, "// @rule RULE:DATA_PRIORITY_ADMIT_MISSING category=process priority=10"),
    R(398, "// @rule RULE:NO_FABRICATED_DATA category=safety priority=30"),
    R(409, "// @rule RULE:CAPABILITIES category=policy"),
    R(417, "// @rule RULE:NO_PRICE_PREDICTION category=safety"),
    R(418, "// @rule RULE:NO_PRECISE_VALUATION category=safety"),
    R(419, "// @rule RULE:NO_UNFAMILIAR_INDUSTRIES category=safety"),
    R(420, "// @rule RULE:NO_RETURN_GUARANTEE category=safety"),
    R(421, "// @rule RULE:NO_BUY_SELL_INSTRUCTIONS category=safety"),
    R(425, "// @rule RULE:IN_SCOPE_TOPICS category=policy"),
    R(432, "// @rule RULE:OUT_OF_SCOPE_TOPICS category=policy"),
    R(441, "// @rule RULE:RECENCY_CAVEAT category=safety"),
]

# ---------------------------------------------------------------- F11  vibe-coding
SPEC["9deacc979622_SKILL.md"] = [
    R(8, "// @rule RULE:WHEN_TO_USE category=process"),
    R(16, "// @rule RULE:FRIENDLY_NAMES_FIRST category=style"),
    R(20, "// @rule RULE:USE_FRIENDLY_NAMES category=style"),
    R(34, "// @rule RULE:NO_ACRONYM_GATE category=style"),
    R(73, "// @rule RULE:DOCS_SOURCE_OF_TRUTH category=policy"),
    R(75, "// @rule RULE:NEW_PROJECTS_ONLY category=policy"),
    # --- branch: beginner
    R(79, "// @condition CONDITION:BEGINNER",
        "{% if user.beginner %}"),
    R(81, "// @rule RULE:EXPLAIN_PLAINLY category=style"),
    R(83, "// @rule RULE:NO_ACRONYMS_FIRST category=style"),
    R(93, "// @rule RULE:EXPLAIN_WHY category=style"),
    R(100, "// @rule RULE:KEEP_EXPLANATIONS_SHORT category=style"),
    A(100, "{% endif %}"),
    R(104, "// @rule RULE:NO_PREMATURE_ARCHITECTURE category=process"),
    R(106, "// @rule RULE:ASK_ONLY_RELEVANT category=process"),
    R(108, "// @rule RULE:CLARIFY_LIST category=process"),
    R(120, "// @rule RULE:RECORD_UNCERTAINTY category=process"),
    R(122, "// @rule RULE:URD_EXIT_CRITERIA category=process priority=30",
        "// @breakpoint BREAKPOINT:URD_GATE"),
    R(133, "// @rule RULE:OCKHAM category=policy"),
    R(135, "// @rule RULE:CONTENT_MUST_SERVE category=policy"),
    R(146, "// @rule RULE:AVOID_BLOAT category=policy"),
    R(156, "// @rule RULE:PARKING_LOT category=process"),
    R(158, "// @rule RULE:OCKHAM_CHECK category=process priority=20"),
    R(182, "// @rule RULE:MATRIX_INTERPRETATION category=policy"),
    R(188, "// @rule RULE:RETRY_ON_COUPLING category=process"),
    R(190, "// @rule RULE:COUPLING_RETRY_LOOP category=process"),
    R(201, "// @rule RULE:RECORD_RETRIES category=format"),
    R(203, "// @rule RULE:RECORD_ACCEPTED_COUPLING category=format"),
    R(212, "// @rule RULE:NO_FAKE_DECOMPOSITION category=policy"),
    R(220, "// @rule RULE:DOCS_BEFORE_WIKI category=process priority=20"),
    R(221, "// @rule RULE:WIKI_NO_NEW_REQUIREMENTS category=policy"),
    R(222, "// @rule RULE:WIKI_NO_OVERRIDE category=policy"),
    R(223, "// @rule RULE:WIKI_CONFLICT_ISSUE category=process"),
    R(224, "// @rule RULE:SHORT_WIKI_PAGES category=format"),
    R(225, "// @rule RULE:ONE_QUESTION_PER_PAGE category=format"),
    R(229, "// @rule RULE:INIT_BEFORE_IMPL category=process priority=20"),
    R(241, "// @rule RULE:STACK_USER_CHOICE category=process priority=40"),
    R(242, "// @rule RULE:STACK_DEFAULT_PYTHON category=process priority=30"),
    R(243, "// @rule RULE:STACK_STATIC_SITE category=process priority=20"),
    R(244, "// @rule RULE:STACK_ASK category=process priority=10"),
    R(246, "// @rule RULE:PYTHON_DEFAULTS category=process"),
    R(258, "// @rule RULE:UV_INIT category=process"),
    R(260, "// @rule RULE:UV_APP_COMMANDS category=process"),
    R(269, "// @rule RULE:UV_LIB_COMMANDS category=process"),
    R(277, "// @rule RULE:NO_SPECULATIVE_DEPS category=policy"),
    R(281, "// @rule RULE:CHECKPOINT_EVERY_SLICE category=process"),
    R(283, "// @rule RULE:CHECKPOINT_STEPS category=process"),
    R(307, "// @rule RULE:NEVER_COMMIT_SECRETS category=safety priority=30"),
    R(308, "// @rule RULE:NEVER_MERGE_FAILING category=safety priority=30"),
    R(309, "// @rule RULE:NEVER_PUSH_LOCAL_ONLY category=safety priority=30"),
    R(310, "// @rule RULE:ASK_BEFORE_FIRST_PUSH category=safety priority=30",
        "// @breakpoint BREAKPOINT:BEFORE_FIRST_PUSH"),
    R(311, "// @rule RULE:NO_REMOTE_LOCAL_COMMITS category=process"),
    R(312, "// @rule RULE:OTHER_PLATFORMS category=process"),
    R(331, "// @rule RULE:MERGE_POLICY category=process"),
    R(342, "// @rule RULE:STABLE_IDS category=format"),
    R(370, "// @rule RULE:MAINTAIN_TRACE category=process"),
    R(385, "// @rule RULE:PROPAGATE_CHANGES category=process",
        "// @breakpoint BREAKPOINT:STOP_IF_UNCERTAIN"),
    R(389, "// @rule RULE:SMALLEST_DOC_SET category=policy"),
    R(393, "// @rule RULE:LEVEL_SIMPLE category=process"),
    R(407, "// @rule RULE:LEVEL_STANDARD category=process"),
    R(423, "// @rule RULE:LEVEL_STRICT category=policy"),
    R(436, "// @rule RULE:MODE_EXPLAIN_PLAINLY category=process"),
    R(460, "// @rule RULE:MODE_INIT_DOCS category=process"),
    R(478, "// @rule RULE:MODE_INIT_CODING category=process"),
    R(512, "// @rule RULE:MODE_GIT_CHECKPOINT category=process"),
    R(541, "// @rule RULE:MODE_DISCOVER_URD category=process"),
    R(566, "// @rule RULE:MODE_ANALYZE_ADD category=process"),
    R(590, "// @rule RULE:MODE_DESIGN_MDD category=process"),
    R(613, "// @rule RULE:MODE_WRITE_TDD category=process"),
    R(636, "// @rule RULE:MODE_PLAN_RMD category=process"),
    R(658, "// @rule RULE:MODE_COMPILE_WIKI category=process"),
    R(680, "// @rule RULE:MODE_UPDATE_DOCS category=process"),
    R(708, "// @rule RULE:MINIMAL_REGENERATION category=policy priority=30"),
    R(712, "// @rule RULE:SHIPPING_GATE category=process priority=20",
        "// @breakpoint BREAKPOINT:SHIPPING_GATE"),
    R(735, "// @rule RULE:BE_DIRECT category=style"),
    R(736, "// @rule RULE:FRIENDLY_NAMES_WITH_BEGINNERS category=style"),
    R(737, "// @rule RULE:FEW_HIGH_VALUE_QUESTIONS category=process"),
    R(738, "// @rule RULE:LABEL_ASSUMPTIONS category=format"),
    R(739, "// @rule RULE:EXPLAIN_REJECTIONS category=style"),
    R(740, "// @rule RULE:SMALL_UPDATES category=process"),
    R(741, "// @rule RULE:NO_SLOGANS category=style"),
]

# ---------------------------------------------------------------- F12  citedy seo
SPEC["ccb5349ebdcc_SKILL.md"] = [
    R(45, "// @rule RULE:ROLE category=style"),
    R(58, "// @rule RULE:WHEN_TO_USE category=process"),
    # --- branch: no saved API key
    R(84, "// @condition CONDITION:API_KEY_MISSING",
        "{% if not agent.api_key %}"),
    R(88, "// @rule RULE:REGISTER_SCRIPT category=process priority=20"),
    R(96, "// @rule RULE:REGISTER_API_DIRECT category=process priority=10"),
    R(117, "// @rule RULE:ASK_HUMAN_APPROVE category=process",
        "// @breakpoint BREAKPOINT:AWAIT_APPROVAL"),
    R(124, "// @rule RULE:STORE_API_KEY category=safety"),
    R(128, "// @rule RULE:FETCH_REFERRAL category=process"),
    R(139, "// @rule RULE:SAVE_REFERRAL_URL category=process"),
    A(139, "{% endif %}"),
    R(147, "// @rule RULE:WORKFLOW_URL_TO_ARTICLE category=process"),
    R(155, "// @rule RULE:WORKFLOW_SCOUT category=process"),
    R(164, "// @rule RULE:WORKFLOW_SESSION category=process"),
    R(172, "// @rule RULE:WORKFLOW_INGEST category=process"),
    R(180, "// @rule RULE:WORKFLOW_GSC category=process"),
    R(187, "// @rule RULE:GSC_NOT_CONNECTED category=process"),
    R(189, "// @rule RULE:PATH_SELECTION category=process"),
    R(268, "// @rule RULE:NO_OFFPAGE_SEO category=policy"),
    R(269, "// @rule RULE:SYNCHRONOUS_GENERATION category=process"),
    R(270, "// @rule RULE:ONE_SESSION_PER_TENANT category=policy"),
    R(271, "// @rule RULE:PUBLISH_ONLY_CONNECTED category=policy"),
    R(272, "// @rule RULE:API_ONLY category=policy"),
    R(273, "// @rule RULE:RATE_AND_CREDIT_LIMITS category=policy"),
    R(279, "// @rule RULE:AUTH_HEADER category=safety"),
    R(399, "// @rule RULE:USE_ARTICLE_URL category=policy priority=30"),
    R(401, "// @rule RULE:AUTOPUBLISH_SEMANTICS category=policy"),
    R(779, "// @rule RULE:AVATAR_APPROVAL category=process priority=20",
        "// @breakpoint BREAKPOINT:AVATAR_APPROVAL"),
    R(848, "// @rule RULE:TIKTOK_PRIVACY category=safety priority=30"),
    R(1225, "// @rule RULE:STORE_WEBHOOK_SECRET category=safety priority=30"),
    R(1298, "// @rule RULE:VERIFY_SIGNATURE category=safety"),
    R(1343, "// @rule RULE:RATE_LIMIT_BACKOFF category=process"),
    R(1349, "// @rule RULE:REPLY_USER_LANGUAGE category=format"),
    R(1350, "// @rule RULE:ANNOUNCE_COST category=process"),
    R(1351, "// @rule RULE:AUTO_POLL category=process"),
    R(1352, "// @rule RULE:READABLE_SUMMARY category=format"),
    R(1353, "// @rule RULE:HIGHLIGHT_TOP5 category=format"),
    R(1354, "// @rule RULE:SHOW_ARTICLE_FIELDS category=format"),
    R(1355, "// @rule RULE:SHOW_ADAPTATION_FIELDS category=format"),
    R(1356, "// @rule RULE:SHOW_SESSION_FIELDS category=format"),
    R(1357, "// @rule RULE:WARN_LOW_BALANCE category=safety"),
    R(1358, "// @rule RULE:INCLUDE_REFERRAL category=policy"),
    R(1359, "// @rule RULE:EXPLAIN_ERRORS category=style"),
    R(1363, "// @rule RULE:ERROR_HANDLING category=process"),
    R(1377, "// @rule RULE:REFERRAL_USAGE category=policy"),
    R(1383, "// @rule RULE:HEARTBEAT category=process"),
]


TEMPLATE_PREFIXES = ("// @", "{% if ", "{% else %}", "{% endif %}", "{% for ", "{% endfor %}")


def is_inserted(line):
    s = line.strip()
    return any(s.startswith(p) or s == p for p in TEMPLATE_PREFIXES)


def build(fname, spec, review):
    src_path = os.path.join(SRC, fname)
    with open(src_path, encoding="utf-8") as fh:
        src = fh.read()
    lines = src.split("\n")
    # src.split("\n") yields a trailing "" if file ends with newline
    before = {}
    after = {}
    for n, pos, payload in spec:
        d = before if pos == "before" else after
        d.setdefault(n, []).extend(payload)

    out = []
    for i, line in enumerate(lines, start=1):
        for a in before.get(i, []):
            out.append(a)
            review.append((fname, i, a, line))
        out.append(line)
        for a in after.get(i, []):
            out.append(a)
            review.append((fname, i, "AFTER " + a, line))

    text = "\n".join(out)

    # fidelity check: stripping inserted lines must reproduce the source exactly
    recovered = "\n".join(l for l in text.split("\n") if not is_inserted(l))
    assert recovered == src, f"FIDELITY FAILURE in {fname}"

    stem = fname[:-3] if fname.endswith(".md") else fname
    dest = os.path.join(OUT, stem + ".prompt.rtpl")
    os.makedirs(OUT, exist_ok=True)
    with open(dest, "w", encoding="utf-8") as fh:
        fh.write(text)
    return dest, len(lines) - (1 if lines and lines[-1] == "" else 0)


def main():
    review = []
    stats = []
    for fname, spec in SPEC.items():
        dest, nlines = build(fname, spec, review)
        txt = open(dest, encoding="utf-8").read()
        rules = txt.count("// @rule ")
        conds = txt.count("// @condition ")
        bps = txt.count("// @breakpoint ")
        ifs = txt.count("{% if ")
        elses = txt.count("{% else %}")
        stats.append((fname, nlines, rules, conds, bps, ifs, elses))
    print(f"{'file':30s} {'lines':>6s} {'rules':>6s} {'cond':>5s} {'bp':>4s} {'if':>4s} {'else':>5s}")
    for s in stats:
        print(f"{s[0]:30s} {s[1]:6d} {s[2]:6d} {s[3]:5d} {s[4]:4d} {s[5]:4d} {s[6]:5d}")
    if "--review" in sys.argv:
        cur = None
        for fname, n, ann, line in review:
            if fname != cur:
                print("\n===== " + fname)
                cur = fname
            print(f"  {n:5d}  {ann:70s} || {line[:90]}")


if __name__ == "__main__":
    main()
