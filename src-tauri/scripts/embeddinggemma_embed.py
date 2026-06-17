#!/usr/bin/env python
"""HalluScribe - local EmbeddingGemma helper."""

import argparse
import json
import sys

from sentence_transformers import SentenceTransformer


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--mode", choices=("query", "document"), required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    texts = json.load(sys.stdin)
    if not isinstance(texts, list) or not all(isinstance(item, str) for item in texts):
        raise ValueError("stdin must be a JSON array of strings")

    model = SentenceTransformer(args.model_dir)
    if args.mode == "query":
        vectors = model.encode_query(texts, normalize_embeddings=True)
    else:
        vectors = model.encode_document(texts, normalize_embeddings=True)

    json.dump(vectors.tolist(), sys.stdout)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
