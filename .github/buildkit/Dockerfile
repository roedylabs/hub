# BuildKit image for docker/setup-buildx-action (driver-opts) in ci.yml and
# release.yml: the builder that runs the Dockerfile and its built-in frontend.
# Never built; the file only exists so that Dependabot (docker,
# .github/dependabot.yml) keeps tag and digest up to date, and
# .github/actions/setup-buildx reads the FROM line. Taken over from Roedy Mail
# (../mail).
FROM moby/buildkit:v0.34.2@sha256:c5f8862f52499773af85bc07695b2379ef08e8002352b0df2c4864c91d78dabe
