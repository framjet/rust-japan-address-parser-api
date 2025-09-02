# Japanese Address Parser API Makefile

.PHONY: help build test clean run docker-build docker-run format lint check audit dev install-tools generate-docker-version

# Default target
help: ## Show this help message
	@echo "Available targets:"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}'

# Development
install-tools: ## Install development tools
	cargo install cargo-watch cargo-audit cargo-llvm-cov

dev: ## Run in development mode with auto-reload
	cargo watch -x run

run: ## Run the application
	cargo run

build: ## Build the application
	cargo build

build-release: ## Build the application in release mode
	cargo build --release

# Testing
test: ## Run all tests
	cargo test

test-verbose: ## Run tests with verbose output
	cargo test -- --nocapture

coverage: ## Generate test coverage report
	cargo llvm-cov --html --open

# Code quality
format: ## Format the code
	cargo fmt

format-check: ## Check code formatting
	cargo fmt --check

lint: ## Run clippy linter
	cargo clippy --all-targets --all-features -- -D warnings

audit: ## Run security audit
	cargo audit

check: format-check lint test ## Run all checks

# Docker
docker-build: ## Build Docker image
	docker build -t framjet/japanese-address-parser-api:latest .

docker-build-multiarch: ## Build multi-architecture Docker images
	docker buildx build --platform linux/amd64,linux/arm64 -t framjet/japanese-address-parser-api:latest .

docker-run: ## Run Docker container
	docker run -p 3000:3000 framjet/japanese-address-parser-api:latest

docker-compose-up: ## Start with docker-compose
	docker-compose up -d

docker-compose-down: ## Stop docker-compose services
	docker-compose down

docker-compose-monitoring: ## Start with monitoring stack
	docker-compose --profile monitoring up -d

# Utilities
clean: ## Clean build artifacts
	cargo clean
	docker system prune -f
	rm -f .server.pid

generate-docker-version: ## Generate Docker version file
	@echo "docker_image_tag=v$(shell grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')" > versions
	@echo "docker_image_name=framjet/japanese-address-parser-api" >> versions
	@echo "build_date=$(shell date -u +'%Y-%m-%dT%H:%M:%SZ')" >> versions
	@echo "git_commit=$(shell git rev-parse HEAD)" >> versions

# Release
prepare-release: check ## Prepare for release
	@echo "Preparing release..."
	@echo "Current version: $(shell grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')"
	@echo "Don't forget to:"
	@echo "  1. Update CHANGELOG.md"
	@echo "  2. Commit changes"
	@echo "  3. Create and push git tag"

# Performance testing
bench: build-release
	 @bash -eu -o pipefail -c '\
	   echo "Starting server for benchmark..."; \
	   ./target/release/rust-japan-address-parser-api & APP_PID=$$!; \
	   trap "kill $$APP_PID 2>/dev/null || true; wait $$APP_PID 2>/dev/null || true" EXIT; \
	   sleep 3; \
	   echo "Running benchmark..."; \
	   /usr/bin/time bash -c "for i in {1..1000}; do curl -s \"http://localhost:3000/parse?address=東京都渋谷区神宮前1-1-1\" > /dev/null; done"; \
	   echo "Stopping server..."; \
	 '

load-test: build-release
	 @bash -eu -o pipefail -c '\
	   ./target/release/rust-japan-address-parser-api & APP_PID=$$!; \
	   trap "kill $$APP_PID 2>/dev/null || true; wait $$APP_PID 2>/dev/null || true" EXIT; \
	   sleep 3; \
	   echo "Running load test (1000 requests, concurrency 10)..."; \
	   ab -n 1000 -c 10 "http://localhost:3000/parse?address=東京都渋谷区神宮前1-1-1"; \
	 '
