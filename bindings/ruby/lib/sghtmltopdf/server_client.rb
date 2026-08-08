# frozen_string_literal: true

require "net/http"
require "uri"

module Sghtmltopdf
  class ServerError < Error; end

  # HTTPサーバモード(`sghtmltopdf server`)へ変換を委譲するクライアント。
  #
  #   Sghtmltopdf.configure { |c| c.server_url = "http://pdf.internal:8080" }
  #   pdf = Sghtmltopdf.render(html, page_size: "A4")
  class ServerClient
    DEFAULT_OPEN_TIMEOUT = 5
    DEFAULT_READ_TIMEOUT = 120

    # 一度に読むチャンクの目安。`?stream=1`のときはこの単位でブロックへ渡る。
    CHUNK_SIZE = 64 * 1024

    attr_reader :uri, :open_timeout, :read_timeout

    # @param url [String] サーバのベースURL(`http://host:port`)
    def initialize(url, open_timeout: nil, read_timeout: nil)
      @uri = parse(url)
      @open_timeout = (open_timeout || DEFAULT_OPEN_TIMEOUT).to_f
      @read_timeout = (read_timeout || DEFAULT_READ_TIMEOUT).to_f
    end

    # HTMLをPDFへ変換する。
    #
    # ブロックを渡すと`?stream=1`(chunked transfer encoding)を使い、
    # サーバがページを確定したそばからチャンクを渡す。ブロックが無ければ
    # PDF全体をStringで返す。
    def render(html, options, &block)
      request = build_request(html, options, stream: !block.nil?)
      pdf = nil
      start do |http|
        # `request`はブロック付きだとレスポンスオブジェクトを返すので、
        # 結果は外の変数で受ける。
        http.request(request) do |response|
          ensure_success!(response)
          if block
            response.read_body { |chunk| block.call(chunk.b) }
          else
            pdf = read_all(response)
          end
        end
      end
      pdf
    end

    # 変換結果を`path`へ書き出す。途中で失敗しても壊れたPDFを残さないよう、
    # 一時ファイルへ書いてからrenameする(ネイティブ拡張の`FileSink`と同じ
    # 挙動に揃えている)。
    def render_to_file(html, options, path)
      tmp = "#{path}.#{Process.pid}.tmp"
      begin
        File.open(tmp, "wb") do |file|
          render(html, options) { |chunk| file.write(chunk) }
        end
      rescue SystemCallError => e
        File.unlink(tmp) if File.exist?(tmp)
        raise InputError, "#{path}への書き出しに失敗しました: #{e.message}"
      rescue StandardError
        File.unlink(tmp) if File.exist?(tmp)
        raise
      end
      File.rename(tmp, path)
      nil
    end

    private

    def parse(url)
      uri = URI.parse(url.to_s)
      unless uri.is_a?(URI::HTTP) && uri.host
        raise ArgumentError, "server_urlにはhttp(s)のURLを指定してください: #{url.inspect}"
      end

      uri
    end

    def build_request(html, options, stream:)
      query = Options.to_query(options)
      query = stream ? [query, "stream=1"].reject(&:empty?).join("&") : query
      target = uri.dup
      target.path = "/pdf"
      target.query = query.empty? ? nil : query

      request = Net::HTTP::Post.new(target)
      request["Content-Type"] = "text/html; charset=utf-8"
      request.body = html.to_s.b
      request
    end

    def start(&block)
      Net::HTTP.start(
        uri.host, uri.port,
        use_ssl: uri.scheme == "https",
        open_timeout: open_timeout,
        read_timeout: read_timeout,
        &block
      )
    rescue Net::OpenTimeout, Net::ReadTimeout => e
      raise ServerError, "#{base}への接続がタイムアウトしました: #{e.class}"
    rescue SocketError, SystemCallError, IOError, OpenSSL::SSL::SSLError => e
      raise ServerError, "#{base}への接続に失敗しました: #{e.message}"
    end

    # エラー応答の本文は`text/plain`の日本語メッセージ(CLIと同じ文言)。
    def ensure_success!(response)
      return if response.is_a?(Net::HTTPOK)

      message = read_all(response).force_encoding(Encoding::UTF_8).strip
      raise error_class(response), "#{base}: #{message}"
    end

    def error_class(response)
      case response.code.to_i
      when 400 then UsageError
      when 413 then InputError
      when 500 then RenderError
      # 404/405はパスやメソッドの間違い＝相手がsghtmltopdfのサーバでない
      # 可能性が高い。503/504はキュー溢れ・キュー待ちのタイムアウト。
      else ServerError
      end
    end

    def read_all(response)
      buffer = +""
      response.read_body { |chunk| buffer << chunk }
      buffer.b
    end

    def base
      "#{uri.scheme}://#{uri.host}:#{uri.port}"
    end
  end
end
