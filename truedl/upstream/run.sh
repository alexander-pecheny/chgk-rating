set -e
python -m pip install -r requirements.txt
python -m truedl.01_download_tournament_info
python -m truedl.02_download_results_and_controversials --date-from 2019-01-01 --date-to 2019-12-31
python -m truedl.04_download_releases
python -m truedl.05_make_dl_db.py
python -m truedl.06_calculate_truedl.py
