build:
	@cargo build

test:
	@cargo nextest run --all-features

test-conformance-verapdf:
	@if [ -z "$$PDFV_VERAPDF_CORPUS_DIR" ]; then \
		echo "PDFV_VERAPDF_CORPUS_DIR must point to a veraPDF-corpus checkout"; \
		exit 2; \
	fi
	@cargo test -p pdfv --test verapdf_corpus -- --ignored --nocapture

check-agent-sync:
	@cmp -s CLAUDE.md AGENTS.md || { \
		echo "AGENTS.md must stay in sync with CLAUDE.md"; \
		echo "Update both files with the same shared project instructions."; \
		exit 1; \
	}
	@tmp_dir=$$(mktemp -d); \
	trap 'rm -rf "$$tmp_dir"' EXIT; \
	cp -R .claude/skills "$$tmp_dir/expected-skills"; \
	find "$$tmp_dir/expected-skills" -name SKILL.md -exec perl -0pi -e 's/CLAUDE\.md/AGENTS.md/g; s/Claude/Codex/g; s/claude/codex/g' {} +; \
	diff -ru --exclude agents "$$tmp_dir/expected-skills" .agents/skills || { \
		echo "Codex skills must stay in sync with Claude skills after Claude-to-Codex renaming."; \
		echo "Update .claude/skills first, then mirror the shared content into .agents/skills."; \
		exit 1; \
	}

release:
	@cargo release tag --execute
	@git cliff -o CHANGELOG.md
	@git commit -a -n -m "Update CHANGELOG.md" || true
	@git push origin master
	@cargo release push --execute

update-submodule:
	@git submodule update --init --recursive --remote

generate-profiles:
	@cargo run -p pdfv-core --example generate_profiles -- crates/core/src/generated_profiles.rs

.PHONY: build test test-conformance-verapdf check-agent-sync release update-submodule generate-profiles
