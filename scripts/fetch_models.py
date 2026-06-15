#!/usr/bin/env python3
import sys
import argparse
import urllib.request
import urllib.error
import json


def mask_api_key(api_key):
    if len(api_key) <= 8:
        return "*" * len(api_key)
    return f"{api_key[:4]}...{api_key[-4:]}"


def render_configuration_guide(base_uri, api_key, api_type, models):
    return "\n".join([
        "",
        "=" * 50,
        "💡 【小白配置指南】",
        "=" * 50,
        "如果这是给 AI 工具配置，请按以下方式填写：",
        f"1. 选择提供商: 选择【{api_type}】或者开启【兼容模式/Custom OpenAI】",
        f"2. Base URL: 填写 \033[1;32m{base_uri.rstrip('/')}/v1\033[0m",
        f"3. API Key: 填写 \033[1;32m{mask_api_key(api_key)}\033[0m",
        f"4. 自定义模型名称: 随便挑上面列表里的一个（比如 \033[1;32m{models[0]}\033[0m）填入自定义模型列表即可",
        "=" * 50,
    ])


def fetch_models(base_uri, api_key):
    # Normalize base_uri by stripping the trailing slash and /v1 so paths can be appended
    base_uri = base_uri.rstrip('/')
    if base_uri.endswith('/v1'):
        base_uri = base_uri[:-3]
    
    models = []
    
    # 1. Try the OpenAI format
    openai_url = f"{base_uri}/v1/models"
    req_openai = urllib.request.Request(openai_url)
    req_openai.add_header('Authorization', f'Bearer {api_key}')
    req_openai.add_header('Accept', 'application/json')
    
    try:
        with urllib.request.urlopen(req_openai, timeout=10) as response:
            data = json.loads(response.read().decode('utf-8'))
            if 'data' in data:
                print("✅ 探测到 OpenAI 格式 API")
                models = [model.get('id', model) if isinstance(model, dict) else str(model) for model in data['data']]
                return "OpenAI", models
    except urllib.error.URLError as e:
        openai_err = str(e)
        if hasattr(e, 'read'):
            try:
                openai_err += f" (响应: {e.read().decode('utf-8')[:100]})"
            except Exception:
                pass
    except Exception as e:
        openai_err = str(e)
        
    # 2. Try the Anthropic format
    anthropic_url = f"{base_uri}/v1/models"
    req_anthropic = urllib.request.Request(anthropic_url)
    req_anthropic.add_header('x-api-key', api_key)
    req_anthropic.add_header('anthropic-version', '2023-06-01')
    req_anthropic.add_header('Accept', 'application/json')
    
    try:
        with urllib.request.urlopen(req_anthropic, timeout=10) as response:
            data = json.loads(response.read().decode('utf-8'))
            print("✅ 探测到 Anthropic 格式 API")
            if isinstance(data, dict) and 'data' in data:
                models = [model.get('id', model) if isinstance(model, dict) else str(model) for model in data['data']]
            elif isinstance(data, dict) and 'models' in data:
                models = [model.get('id', model) if isinstance(model, dict) else str(model) for model in data['models']]
            elif isinstance(data, list):
                models = [model.get('id', model) if isinstance(model, dict) else str(model) for model in data]
            else:
                models = []
            return "Anthropic", models
    except urllib.error.URLError as e:
        anthropic_err = str(e)
        if hasattr(e, 'read'):
            try:
                anthropic_err += f" (响应: {e.read().decode('utf-8')[:100]})"
            except Exception:
                pass
    except Exception as e:
        anthropic_err = str(e)

    print("❌ 无法获取模型列表，或该 API 地址不支持 /v1/models 端点。")
    print(f"   [OpenAI 尝试错误]: {openai_err}")
    print(f"   [Anthropic 尝试错误]: {anthropic_err}")
    return None, []

def main():
    parser = argparse.ArgumentParser(description="根据 baseuri 和 key 探测并获取支持的模型列表")
    parser.add_argument('baseuri', nargs='?', help='API Base URI (例如: http://172.36.236.44:3010)')
    parser.add_argument('key', nargs='?', help='API Key (例如: sk-...)')
    
    args = parser.parse_args()
    
    import os
    base_uri = args.baseuri or os.environ.get('OPENAI_BASE_URL') or os.environ.get('OPENAI_API_BASE') or os.environ.get('ANTHROPIC_BASE_URL')
    api_key = args.key or os.environ.get('OPENAI_API_KEY') or os.environ.get('ANTHROPIC_API_KEY')
    
    if not base_uri or not api_key:
        print("❌ 未提供 baseuri 或 key，且无法从系统环境变量自动获取 (如 OPENAI_BASE_URL / OPENAI_API_KEY)。")
        print("用法:")
        print("  方式1: 传参调用 ./manage.sh fetch-models <base_uri> <api_key>")
        print("  方式2: 环境变量 export OPENAI_BASE_URL=... && export OPENAI_API_KEY=... && ./manage.sh fetch-models")
        sys.exit(1)
        
    print(f"正在探测 API 端点: {base_uri}")
    api_type, models = fetch_models(base_uri, api_key)
    
    if models:
        print(f"\n✅ 成功！一共获取到 {len(models)} 个支持的模型:")
        for m in sorted(models):
            print(f"  - {m}")
        print(render_configuration_guide(base_uri, api_key, api_type, models))
        
        sys.exit(0)
    else:
        sys.exit(1)

if __name__ == '__main__':
    main()
