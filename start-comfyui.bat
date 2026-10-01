@echo off
rem ComfyUI loads its models from the one models folder (MODELS_DIR, see common\models-env.bat); files still in
rem ComfyUI\models are found too until they are moved (GUI > Models > Move them).
call "%~dp0common\models-env.bat"
cd /d "%~dp0ComfyUI_windows_portable"
.\python_embeded\python.exe -s ComfyUI\main.py --windows-standalone-build --lowvram --models-directory "%MODELS_DIR%" --extra-model-paths-config "%~dp0config\comfyui-old-models.yaml"
pause
