FROM python:3.12-slim

WORKDIR /app

RUN pip install --no-cache-dir kuksa-client==0.4.3

COPY vehicles.json .
COPY seed.py .

CMD ["python", "seed.py"]
