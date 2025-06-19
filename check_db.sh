#!/bin/bash

echo "Checking SurrealDB connection..."

# Test connection with curl
curl -X POST \
  -H "Accept: application/json" \
  -H "Surreal-NS: gitstars" \
  -H "Surreal-DB: stars" \
  -u "root:root" \
  -d "SELECT * FROM repo LIMIT 5" \
  http://localhost:8000/sql

echo ""
echo "Checking count..."
curl -X POST \
  -H "Accept: application/json" \
  -H "Surreal-NS: gitstars" \
  -H "Surreal-DB: stars" \
  -u "root:root" \
  -d "SELECT count() FROM repo GROUP ALL" \
  http://localhost:8000/sql